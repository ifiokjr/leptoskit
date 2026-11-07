import Foundation
import LeptosKitBridge
#if canImport(UIKit)
import UIKit
#endif

/// One portable shortcut the OS may show for this app.
public struct AppShortcutItem: Equatable, Sendable {
    public let id: String
    public let title: String
    public let subtitle: String

    public init(id: String, title: String, subtitle: String) {
        self.id = id
        self.title = title
        self.subtitle = subtitle
    }
}

@MainActor
protocol ShortcutPublisher {
    func set(_ items: [AppShortcutItem]) throws
    func clear() throws
}

/// Latest shortcuts published for the macOS dock menu. The generated host can
/// return these from `applicationDockMenu(_:)` until the host wires the menu
/// automatically.
@MainActor
public enum DockShortcutStore {
    public static var items: [AppShortcutItem] = []
}

@MainActor
private struct SystemShortcutPublisher: ShortcutPublisher {
    func set(_ items: [AppShortcutItem]) throws {
        #if canImport(UIKit)
        UIApplication.shared.shortcutItems = items.map { item in
            UIApplicationShortcutItem(
                type: item.id,
                localizedTitle: item.title,
                localizedSubtitle: item.subtitle.isEmpty ? nil : item.subtitle,
                icon: nil,
                userInfo: nil
            )
        }
        #elseif canImport(AppKit)
        DockShortcutStore.items = items
        #endif
    }

    func clear() throws {
        try set([])
    }
}

/// Publishes static home-screen / dock shortcuts. Launch delivery is recorded by
/// the host through ``noteLaunch(id:)`` when the OS activates a shortcut.
@MainActor
public final class AppShortcutsPlugin: NativePlugin {
    public let name = "app-shortcuts"
    public let methods: [UInt32] = [0, 1, 2]

    private let publisher: any ShortcutPublisher
    private var closed = false
    private var launchId: String?

    public init() {
        publisher = SystemShortcutPublisher()
    }

    init(publisher: any ShortcutPublisher) {
        self.publisher = publisher
    }

    /// Records that the OS launched or activated the app through this id.
    public func noteLaunch(id: String) throws {
        guard !closed else { throw PluginError.cancelled }
        try Self.validateId(id)
        launchId = id
    }

    public func invoke(method: UInt32, payload: NativeBuffer) async throws -> Data {
        guard !closed else { throw PluginError.cancelled }
        guard methods.contains(method) else { throw PluginError.unavailable }

        switch method {
        case 0:
            try publisher.set(try Self.decodeList(payload))
            return Data()
        case 1:
            guard payload.count == 0 else { throw PluginError.invalidArgument }
            try publisher.clear()
            return Data()
        default:
            guard payload.count == 0 else { throw PluginError.invalidArgument }
            let id = launchId
            launchId = nil
            return try Self.encodeLaunch(id)
        }
    }

    public func close() {
        closed = true
    }

    private static func decodeList(_ payload: NativeBuffer) throws -> [AppShortcutItem] {
        try payload.withUnsafeBytes { bytes in
            var offset = 0
            let count = try readU32(bytes, &offset)
            guard count <= 8 else { throw PluginError.tooLarge }
            var items: [AppShortcutItem] = []
            items.reserveCapacity(Int(count))
            var seen = Set<String>()
            for _ in 0..<count {
                let id = try readString(bytes, &offset, max: 64)
                let title = try readString(bytes, &offset, max: 128)
                let subtitle = try readString(bytes, &offset, max: 128)
                try validateId(id)
                try validateText(title, allowEmpty: false)
                try validateText(subtitle, allowEmpty: true)
                guard seen.insert(id).inserted else { throw PluginError.invalidArgument }
                items.append(AppShortcutItem(id: id, title: title, subtitle: subtitle))
            }
            guard offset == bytes.count else { throw PluginError.invalidArgument }
            return items
        }
    }

    private static func encodeLaunch(_ id: String?) throws -> Data {
        let value = id ?? ""
        if !value.isEmpty { try validateId(value) }
        var data = Data()
        var length = UInt32(value.utf8.count).littleEndian
        withUnsafeBytes(of: &length) { data.append(contentsOf: $0) }
        data.append(contentsOf: value.utf8)
        return data
    }

    private static func validateId(_ id: String) throws {
        guard !id.isEmpty, id.utf8.count <= 64 else { throw PluginError.invalidArgument }
        guard id.unicodeScalars.allSatisfy({
            CharacterSet.alphanumerics.contains($0) || "-_.".unicodeScalars.contains($0)
        }) else {
            throw PluginError.invalidArgument
        }
    }

    private static func validateText(_ value: String, allowEmpty: Bool) throws {
        guard allowEmpty || !value.isEmpty, value.utf8.count <= 128 else {
            throw PluginError.invalidArgument
        }
        guard value.unicodeScalars.allSatisfy({ !CharacterSet.controlCharacters.contains($0) }) else {
            throw PluginError.invalidArgument
        }
    }

    private static func readU32(_ bytes: UnsafeRawBufferPointer, _ offset: inout Int) throws -> UInt32 {
        guard offset + 4 <= bytes.count else { throw PluginError.invalidArgument }
        let value = bytes.loadUnaligned(fromByteOffset: offset, as: UInt32.self).littleEndian
        offset += 4
        return value
    }

    private static func readString(
        _ bytes: UnsafeRawBufferPointer,
        _ offset: inout Int,
        max: Int
    ) throws -> String {
        let length = Int(try readU32(bytes, &offset))
        guard length <= max, offset + length <= bytes.count else { throw PluginError.invalidArgument }
        let slice = bytes[offset..<(offset + length)]
        offset += length
        guard let value = String(bytes: slice, encoding: .utf8) else { throw PluginError.invalidArgument }
        return value
    }
}
