import Foundation
import LeptosKitBridge
import UserNotifications

@MainActor
protocol NotificationService {
    func authorization() async -> UNAuthorizationStatus
    func requestAuthorization() async throws
    func add(_ request: UNNotificationRequest) async throws
    func identifiers() async -> [String]
    func remove(_ identifiers: [String])
}

@MainActor
private final class SystemNotificationService: NotificationService {
    private let center = UNUserNotificationCenter.current()
    func authorization() async -> UNAuthorizationStatus { await center.notificationSettings().authorizationStatus }
    func requestAuthorization() async throws { _ = try await center.requestAuthorization(options: [.alert, .sound]) }
    func add(_ request: UNNotificationRequest) async throws { try await center.add(request) }
    func identifiers() async -> [String] {
        let pending = await center.pendingNotificationRequests().map(\.identifier)
        let delivered = await center.deliveredNotifications().map { $0.request.identifier }

        return pending + delivered
    }
    func remove(_ identifiers: [String]) {
        center.removePendingNotificationRequests(withIdentifiers: identifiers)
        center.removeDeliveredNotifications(withIdentifiers: identifiers)
    }
}

/// Immediate local notifications. Does not replace the app's notification center delegate.
@MainActor
public final class NotificationsPlugin: NativePlugin {
    public let name = "notifications"
    public let methods: [UInt32] = [0, 1, 2, 3, 6]
    private let service: any NotificationService
    private var closed = false
    private var requesting = false
    private struct Pending { let ticket: UUID; var cancelled = false }
    private var pending: [String: Pending] = [:]
    static let prefix = "leptoskit.notifications."

    public init() { service = SystemNotificationService() }
    init(service: any NotificationService) { self.service = service }

    public func invoke(method: UInt32, payload: NativeBuffer) async throws -> Data {
        guard !closed else { throw PluginError.cancelled }
        try Task.checkCancellation()
        switch method {
        case 0, 1:
            guard payload.count == 0 else { throw PluginError.invalidArgument }
            return try await authorization(request: method == 1)
        case 2:
            let content = try payload.withUnsafeBytes { try Content($0) }
            try await show(content)
        case 3:
            let id = try payload.withUnsafeBytes { bytes in
                var reader = try Reader(bytes, maximum: 68)
                let id = try reader.identifier()
                try reader.finish()
                return Self.prefix + id
            }
            pending[id]?.cancelled = true
            service.remove([id])
        case 6:
            guard payload.count == 0 else { throw PluginError.invalidArgument }

            for id in pending.keys { pending[id]?.cancelled = true }
            let identifiers = await service.identifiers().filter { $0.hasPrefix(Self.prefix) }

            guard !closed else { throw PluginError.cancelled }
            service.remove(identifiers)
        default:
            throw PluginError.unavailable
        }

        return Data()
    }

    private func authorization(request: Bool) async throws -> Data {
        if request {
            guard !requesting else { throw PluginError.busy }
            requesting = true
            defer { requesting = false }

            do { try await service.requestAuthorization() } catch { throw mapped(error) }
        }

        let status = await service.authorization()

        guard !closed else { throw PluginError.cancelled }
        try Task.checkCancellation()

        return encode(status)
    }

    /// Delivered notifications remain visible. In-flight posts are cancelled and
    /// removed again after completion if an OS operation could not be aborted.
    public func close() {
        closed = true

        for id in pending.keys { pending[id]?.cancelled = true }
        service.remove(Array(pending.keys))
    }

    private func show(_ content: Content) async throws {
        let id = Self.prefix + content.id

        guard pending[id] == nil, pending.count < 64 else { throw PluginError.busy }
        let ticket = UUID()
        pending[id] = Pending(ticket: ticket)
        defer { pending.removeValue(forKey: id) }
        try await withTaskCancellationHandler {
            let status = await service.authorization()
            try checkPending(id)

            guard (3...5).contains(authorizationValue(status)) else {
                throw PluginError.permissionDenied
            }

            let native = UNMutableNotificationContent()
            native.title = content.title
            native.body = content.body
            // Sound is deliberately absent. Android sound is an explicit channel setting.
            do {
                try await service.add(UNNotificationRequest(identifier: id, content: native, trigger: nil))
            } catch {
                service.remove([id])
                throw mapped(error)
            }

            do { try checkPending(id) } catch {
                service.remove([id])

                throw error
            }
        } onCancel: { [weak self] in
            Task { @MainActor in

                guard let self, self.pending[id]?.ticket == ticket else { return }
                self.pending[id]?.cancelled = true
                self.service.remove([id])
            }
        }
    }

    private func checkPending(_ id: String) throws {
        guard !closed, pending[id]?.cancelled == false else { throw PluginError.cancelled }
        try Task.checkCancellation()
    }

    private func mapped(_ error: any Error) -> any Error {
        if error is CancellationError { return PluginError.cancelled }
        let native = error as NSError

        if native.domain == UNErrorDomain && native.code == UNError.Code.notificationsNotAllowed.rawValue {
            return PluginError.permissionDenied
        }

        return PluginError.native
    }

    private func encode(_ status: UNAuthorizationStatus) -> Data {
        Data([authorizationValue(status), 0, 0, 0])
    }

    private func authorizationValue(_ status: UNAuthorizationStatus) -> UInt8 {
        switch status {
        case .notDetermined: 1
        case .denied: 2
        case .authorized: 3
        case .provisional: 4

        #if os(iOS)
        case .ephemeral: 5
        #endif
        @unknown default: 0
        }
    }
}

private struct Content {
    let id: String
    let title: String
    let body: String
    init(_ bytes: UnsafeRawBufferPointer) throws {
        var reader = try Reader(bytes, maximum: 4496)
        id = try reader.identifier()
        title = try reader.text(maximum: 256, multiline: false, required: true)
        body = try reader.text(maximum: 4096, multiline: true, required: false)
        _ = try reader.identifier() // Validated Android channel; unsupported channel mutation is separate.
        try reader.finish()
    }
}

private struct Reader {
    private let bytes: UnsafeRawBufferPointer
    private var offset = 0
    init(_ bytes: UnsafeRawBufferPointer, maximum: Int) throws {
        guard bytes.count <= maximum else { throw PluginError.invalidArgument }
        self.bytes = bytes
    }
    mutating func identifier() throws -> String {
        let value = try text(maximum: 64, multiline: false, required: true)
        guard value.utf8.allSatisfy({ (48...57).contains($0) || (65...90).contains($0) ||
            (97...122).contains($0) || [45, 46, 95].contains($0) }) else { throw PluginError.invalidArgument }

        return value
    }

    mutating func text(maximum: Int, multiline: Bool, required: Bool) throws -> String {
        guard bytes.count - offset >= 4 else { throw PluginError.invalidArgument }
        let length = Int(UInt32(littleEndian: bytes.loadUnaligned(fromByteOffset: offset, as: UInt32.self)))
        offset += 4

        guard length <= maximum, length <= bytes.count - offset,
              let value = String(bytes: bytes[offset..<(offset + length)], encoding: .utf8) else {
            throw PluginError.invalidArgument
        }
        offset += length

        guard !required || !value.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,

              !value.unicodeScalars.contains(where: { ($0.value <= 31 || (127...159).contains($0.value)) &&
                  !(multiline && ($0.value == 10 || $0.value == 9)) }) else { throw PluginError.invalidArgument }

        return value
    }

    func finish() throws { guard offset == bytes.count else { throw PluginError.invalidArgument } }
}
