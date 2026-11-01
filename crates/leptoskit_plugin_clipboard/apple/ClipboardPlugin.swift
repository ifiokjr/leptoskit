import Foundation
import LeptosKitBridge
#if os(iOS)
import UIKit
#elseif os(macOS)
import AppKit
#endif

@MainActor
public final class ClipboardPlugin: NativePlugin {
    public let name = "clipboard"
    public let methods: [UInt32] = [0, 1, 2]

    public init() {}

    public func invoke(method: UInt32, payload: NativeBuffer) async throws -> Data {
        switch method {
        case 0:
            guard payload.count == 0 else { throw PluginError.invalidArgument }

            return try Self.read()
        case 1:
            try Self.write(payload.utf8())

            return Data()
        case 2:
            guard payload.count == 0 else { throw PluginError.invalidArgument }

            try Self.clear()

            return Data()
        default:
            throw PluginError.unavailable
        }
    }

    /// The clipboard's text, as bytes.
    private static func read() throws -> Data {
        #if os(iOS)
        return Data((UIPasteboard.general.string ?? "").utf8)
        #elseif os(macOS)
        return Data((NSPasteboard.general.string(forType: .string) ?? "").utf8)
        #else
        throw PluginError.unavailable
        #endif
    }

    /// Replaces the clipboard's text.
    private static func write(_ text: String) throws {
        #if os(iOS)
        UIPasteboard.general.string = text
        #elseif os(macOS)
        NSPasteboard.general.clearContents()

        guard NSPasteboard.general.setString(text, forType: .string) else {
            throw PluginError.native
        }
        #else
        throw PluginError.unavailable
        #endif
    }

    /// Empties the clipboard.
    private static func clear() throws {
        #if os(iOS)
        UIPasteboard.general.items = []
        #elseif os(macOS)
        NSPasteboard.general.clearContents()
        #else
        throw PluginError.unavailable
        #endif
    }
}
