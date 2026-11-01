import Foundation
import LeptosKitBridge
#if os(iOS)
import UIKit
#elseif os(macOS)
import AppKit
#endif

@MainActor
public final class LinkingPlugin: NativePlugin {
    public let name = "linking"
    public let methods: [UInt32] = [0, 1, 2]

    public init() {}

    public func invoke(method: UInt32, payload: NativeBuffer) async throws -> Data {
        if method == 2 {
            guard payload.count == 0 else { throw PluginError.invalidArgument }

            #if os(iOS)

            guard let url = URL(string: UIApplication.openSettingsURLString),
                  await UIApplication.shared.open(url) else { throw PluginError.unavailable }

            return Data()
            #else
            throw PluginError.unavailable
            #endif
        }

        guard method <= 1 else { throw PluginError.unavailable }

        guard (1...8192).contains(payload.count) else { throw PluginError.invalidArgument }
        let url = try Self.validatedURL(payload.utf8())

        #if os(iOS)

        if method == 1 {
            return Self.boolean(UIApplication.shared.canOpenURL(url))
        }

        guard await UIApplication.shared.open(url) else { throw PluginError.unavailable }
        #elseif os(macOS)

        if method == 1 {
            return Self.boolean(NSWorkspace.shared.urlForApplication(toOpen: url) != nil)
        }

        guard NSWorkspace.shared.open(url) else { throw PluginError.unavailable }
        #else
        throw PluginError.unavailable
        #endif

        return Data()
    }

    private static func boolean(_ value: Bool) -> Data {
        Data([value ? 1 : 0, 0, 0, 0])
    }

    static func validatedURL(_ value: String) throws -> URL {
        let bytes = Array(value.utf8)

        guard !bytes.isEmpty, bytes.count <= 8192,
              bytes.allSatisfy({ $0 >= 33 && $0 <= 126 && $0 != 92 }) else {
            throw PluginError.invalidArgument
        }

        for index in bytes.indices where bytes[index] == 37 {
            guard index + 2 < bytes.count,
                  let digits = String(bytes: bytes[(index + 1)...(index + 2)], encoding: .utf8),
                  let decoded = UInt8(digits, radix: 16),

                  decoded >= 32, decoded != 127 else { throw PluginError.invalidArgument }
        }

        guard let components = URLComponents(string: value),
              let scheme = components.scheme?.lowercased(), let url = components.url else {
            throw PluginError.invalidArgument
        }
        switch scheme {
        case "http", "https":
            let destination = value.drop(while: { $0 != ":" }).dropFirst()

            guard destination.hasPrefix("//"), !destination.hasPrefix("///"),
                  let host = components.host, !host.isEmpty,
                  components.user == nil, components.password == nil else { throw PluginError.invalidArgument }
        case "mailto", "tel", "sms":
            guard !components.path.isEmpty, !components.path.hasPrefix("/"),
                  components.host == nil else { throw PluginError.invalidArgument }
        default:
            throw PluginError.invalidArgument
        }

        return url
    }
}
