import Foundation
import LeptosKitBridge
import UserNotifications
#if canImport(AppKit)
import AppKit
#endif

@MainActor
protocol BadgeService {
    func get() -> UInt32
    func set(_ count: UInt32) async throws
}

@MainActor
private final class SystemBadgeService: BadgeService {
    private var count: UInt32 = 0

    func get() -> UInt32 { count }

    func set(_ count: UInt32) async throws {
        self.count = count
        #if canImport(AppKit) && !targetEnvironment(macCatalyst)
        NSApp.dockTile.badgeLabel = count == 0 ? nil : "\(count)"
        #else
        try await UNUserNotificationCenter.current().setBadgeCount(Int(count))
        #endif
    }
}

/// Sets the app icon badge count. Visibility still depends on OS focus and
/// notification settings.
@MainActor
public final class BadgePlugin: NativePlugin {
    public let name = "badge"
    public let methods: [UInt32] = [0, 1, 2]

    private static let maxCount: UInt32 = 999_999
    private let service: any BadgeService
    private var closed = false

    public init() {
        service = SystemBadgeService()
    }

    init(service: any BadgeService) {
        self.service = service
    }

    public func invoke(method: UInt32, payload: NativeBuffer) async throws -> Data {
        guard !closed else { throw PluginError.cancelled }
        guard methods.contains(method) else { throw PluginError.unavailable }

        switch method {
        case 0:
            guard payload.count == 0 else { throw PluginError.invalidArgument }
            return Self.encode(service.get())
        case 1:
            try await service.set(try Self.decode(payload))
            return Data()
        default:
            guard payload.count == 0 else { throw PluginError.invalidArgument }
            try await service.set(0)
            return Data()
        }
    }

    public func close() {
        closed = true
    }

    private static func decode(_ payload: NativeBuffer) throws -> UInt32 {
        guard payload.count == 4 else { throw PluginError.invalidArgument }
        let count = payload.withUnsafeBytes { UInt32(littleEndian: $0.loadUnaligned(as: UInt32.self)) }
        guard count <= maxCount else { throw PluginError.invalidArgument }
        return count
    }

    private static func encode(_ count: UInt32) -> Data {
        var value = count.littleEndian
        return withUnsafeBytes(of: &value) { Data($0) }
    }
}
