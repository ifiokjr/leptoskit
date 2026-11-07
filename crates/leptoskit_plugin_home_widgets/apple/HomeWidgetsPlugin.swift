import Foundation
import LeptosKitBridge
import WidgetKit

/// Shared storage and WidgetKit reload surface used by the plugin.
@MainActor
protocol HomeWidgetService {
    var sharedStoreReady: Bool { get }
    func write(kind: String, bytes: Data)
    func read(kind: String) -> Data?
    func remove(kind: String)
    func kinds() -> [String]
    func reload(kind: String?)
}

@MainActor
private final class SystemHomeWidgetService: HomeWidgetService {
    static let prefix = "leptoskit.home-widgets."
    private let defaults: UserDefaults
    let sharedStoreReady: Bool

    init(appGroup: String?) {
        if let appGroup, let suite = UserDefaults(suiteName: appGroup) {
            defaults = suite
            sharedStoreReady = true
        } else {
            defaults = .standard
            sharedStoreReady = false
        }
    }

    func write(kind: String, bytes: Data) {
        defaults.set(bytes, forKey: Self.prefix + kind)
    }

    func read(kind: String) -> Data? {
        defaults.data(forKey: Self.prefix + kind)
    }

    func remove(kind: String) {
        defaults.removeObject(forKey: Self.prefix + kind)
    }

    func kinds() -> [String] {
        defaults.dictionaryRepresentation().keys.compactMap { key in
            guard key.hasPrefix(Self.prefix) else { return nil }
            return String(key.dropFirst(Self.prefix.count))
        }
    }

    func reload(kind: String?) {
        if let kind {
            WidgetCenter.shared.reloadTimelines(ofKind: kind)
        } else {
            WidgetCenter.shared.reloadAllTimelines()
        }
    }
}

/// Publishes snapshot bytes for WidgetKit extensions and asks WidgetCenter to reload.
/// Does not create, pin, or draw a widget; the app still owns its Widget extension.
@MainActor
public final class HomeWidgetsPlugin: NativePlugin {
    public let name = "home-widgets"
    public let methods: [UInt32] = [0, 1, 2, 3, 4, 5]
    private let service: any HomeWidgetService
    private var closed = false

    /// Uses standard UserDefaults when `appGroup` is nil. Widget extensions that
    /// need shared state must pass the App Group identifier.
    public init(appGroup: String? = nil) {
        service = SystemHomeWidgetService(appGroup: appGroup)
    }

    init(service: any HomeWidgetService) {
        self.service = service
    }

    public func close() {
        closed = true
    }

    public func invoke(method: UInt32, payload: NativeBuffer) async throws -> Data {
        guard !closed else { throw PluginError.cancelled }
        try Task.checkCancellation()

        switch method {
        case 0:
            guard payload.count == 0 else { throw PluginError.invalidArgument }
            return capability()
        case 1:
            let snapshot = try payload.withUnsafeBytes { try Snapshot($0) }
            let bytes = payload.withUnsafeBytes { Data($0) }
            service.write(kind: snapshot.kind, bytes: bytes)
            return Data()
        case 2:
            let kind = try payload.withUnsafeBytes { try KindReader($0).kind }
            guard let bytes = service.read(kind: kind) else { throw PluginError.unavailable }
            return bytes
        case 3:
            let kind = try payload.withUnsafeBytes { try KindReader($0).kind }
            service.remove(kind: kind)
            return Data()
        case 4:
            let kind = try payload.withUnsafeBytes { try optionalKind($0) }
            service.reload(kind: kind)
            return Data()
        case 5:
            let kind = try payload.withUnsafeBytes { try KindReader($0).kind }
            _ = kind
            // WidgetKit has no public pin-state query.
            return Data([0, 0, 0, 0])
        default:
            throw PluginError.unavailable
        }
    }

    private func capability() -> Data {
        var flags: UInt32 = 1 | (1 << 1) // supported + can_reload
        if service.sharedStoreReady {
            flags |= 1 << 3
        }
        var data = Data(count: 4)
        withUnsafeBytes(of: flags.littleEndian) { data.replaceSubrange(0..<4, with: $0) }
        return data
    }
}

private func optionalKind(_ bytes: UnsafeRawBufferPointer) throws -> String? {
    if bytes.isEmpty { return nil }
    return try KindReader(bytes).kind
}

private struct KindReader {
    let kind: String
    init(_ bytes: UnsafeRawBufferPointer) throws {
        var reader = try Reader(bytes, maximum: 68)
        kind = try reader.identifier()
        try reader.finish()
    }
}

private struct Snapshot {
    let kind: String
    init(_ bytes: UnsafeRawBufferPointer) throws {
        var reader = try Reader(bytes, maximum: 16 + 64 + 256 + 1024 + 2048)
        kind = try reader.identifier()
        let family = try reader.u32()
        guard (0...4).contains(family) else { throw PluginError.invalidArgument }
        _ = try reader.text(maximum: 256, multiline: false, required: true)
        _ = try reader.text(maximum: 1024, multiline: true, required: false)
        let url = try reader.text(maximum: 2048, multiline: false, required: false)
        if !url.isEmpty {
            guard url.contains(":"),
                  !url.unicodeScalars.contains(where: { $0.value < 32 || $0.value == 127 }) else {
                throw PluginError.invalidArgument
            }
        }
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

    mutating func u32() throws -> UInt32 {
        guard bytes.count - offset >= 4 else { throw PluginError.invalidArgument }
        let value = UInt32(littleEndian: bytes.loadUnaligned(fromByteOffset: offset, as: UInt32.self))
        offset += 4
        return value
    }

    mutating func identifier() throws -> String {
        let value = try text(maximum: 64, multiline: false, required: true)
        guard value.utf8.allSatisfy({
            (48...57).contains($0) || (65...90).contains($0) || (97...122).contains($0)
                || [45, 46, 95].contains($0)
        }) else { throw PluginError.invalidArgument }
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
              !value.unicodeScalars.contains(where: {
                  ($0.value <= 31 || (127...159).contains($0.value))
                      && !(multiline && ($0.value == 10 || $0.value == 9))
              }) else { throw PluginError.invalidArgument }
        return value
    }

    func finish() throws {
        guard offset == bytes.count else { throw PluginError.invalidArgument }
    }
}
