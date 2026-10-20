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
            #if os(iOS)
            return Data((UIPasteboard.general.string ?? "").utf8)
            #elseif os(macOS)
            return Data((NSPasteboard.general.string(forType: .string) ?? "").utf8)
            #else
            throw PluginError.unavailable
            #endif
        case 1:
            let text = try payload.utf8()
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
            return Data()
        case 2:
            guard payload.count == 0 else { throw PluginError.invalidArgument }
            #if os(iOS)
            UIPasteboard.general.items = []
            #elseif os(macOS)
            NSPasteboard.general.clearContents()
            #else
            throw PluginError.unavailable
            #endif
            return Data()
        default:
            throw PluginError.unavailable
        }
    }
}
