import Foundation
import LeptosKitBridge
#if os(iOS)
import UIKit
#elseif os(macOS)
import AppKit
#endif

/// Outbound links only; incoming deep links belong to the app host lifecycle.
@MainActor
public final class LinkingPlugin: NativePlugin {
    public let name = "linking"
    public let methods: [UInt32] = [0, 1, 2]

    /// The longest payload that can still be a usable URL.
    private static let maxURLBytes = 8192

    /// The printable ASCII range a URL is allowed to carry literally.
    private static let firstPrintable: UInt8 = 33
    private static let lastPrintable: UInt8 = 126

    /// Escapes at or below this code hide a control character.
    private static let firstVisible: UInt8 = 32

    /// The delete code never belongs in a link.
    private static let deleteCode: UInt8 = 127

    /// Bytes a URL can never carry literally.
    private static let backslash: UInt8 = 92
    private static let percentSign: UInt8 = 37

    /// An escape is two hexadecimal digits.
    private static let hexRadix = 16

    public init() {}

    public func invoke(method: UInt32, payload: NativeBuffer) async throws -> Data {
        switch method {
        case 2:
            guard payload.count == 0 else { throw PluginError.invalidArgument }

            try await Self.openSystemSettings()

            return Data()
        case 1:
            let url = try Self.link(from: payload)

            return Self.boolean(try Self.handlerExists(for: url))
        case 0:
            let url = try Self.link(from: payload)

            try await Self.open(url)

            return Data()
        default:
            throw PluginError.unavailable
        }
    }

    /// Validates an untrusted payload as a link before any launcher sees it.
    private static func link(from payload: NativeBuffer) throws -> URL {
        guard (1...maxURLBytes).contains(payload.count) else { throw PluginError.invalidArgument }

        return try validatedURL(payload.utf8())
    }

    /// Encodes a boolean as the one-byte truth and three padding bytes.
    private static func boolean(_ value: Bool) -> Data {
        Data([value ? 1 : 0, 0, 0, 0])
    }

    /// Parses an absolute http(s), mailto, tel or sms link.
    static func validatedURL(_ value: String) throws -> URL {
        let bytes = Array(value.utf8)

        try validateEncoded(bytes)

        guard let components = URLComponents(string: value),
              let scheme = components.scheme?.lowercased(),
              let url = components.url else { throw PluginError.invalidArgument }

        switch scheme {
        case "http", "https": try validateWebDestination(value, components)
        case "mailto", "tel", "sms": try validateContactDestination(components)
        default: throw PluginError.invalidArgument
        }

        return url
    }

    /// Refuses an empty, oversized or non-ASCII payload, then its escapes.
    private static func validateEncoded(_ bytes: [UInt8]) throws {
        guard !bytes.isEmpty, bytes.count <= maxURLBytes,
              bytes.allSatisfy({ $0 >= firstPrintable && $0 <= lastPrintable && $0 != backslash }) else {
            throw PluginError.invalidArgument
        }

        for index in bytes.indices where bytes[index] == percentSign {
            try validateEscape(bytes, at: index)
        }
    }

    /// Rejects an escape that would decode to a control character downstream.
    private static func validateEscape(_ bytes: [UInt8], at index: Int) throws {
        guard index + 2 < bytes.count,
              let digits = String(bytes: bytes[(index + 1)...(index + 2)], encoding: .utf8),
              let decoded = UInt8(digits, radix: hexRadix),
              decoded >= firstVisible, decoded != deleteCode else { throw PluginError.invalidArgument }
    }

    /// A web link needs a host in authority position and no credentials, so
    /// `https:example.com` and `https://user:secret@example.com` stay invalid.
    private static func validateWebDestination(_ value: String, _ components: URLComponents) throws {
        let destination = value.drop(while: { $0 != ":" }).dropFirst()

        guard destination.hasPrefix("//"), !destination.hasPrefix("///"),
              let host = components.host, !host.isEmpty,
              components.user == nil, components.password == nil else { throw PluginError.invalidArgument }
    }

    /// Contact links carry their target in the path, so `tel:` and
    /// `sms://host` stay invalid.
    private static func validateContactDestination(_ components: URLComponents) throws {
        guard !components.path.isEmpty, !components.path.hasPrefix("/"), components.host == nil else {
            throw PluginError.invalidArgument
        }
    }

    #if os(iOS)
    /// Opens this application's page in the Settings app.
    private static func openSystemSettings() async throws {
        guard let url = URL(string: UIApplication.openSettingsURLString),
              await UIApplication.shared.open(url) else { throw PluginError.unavailable }
    }

    /// Asks iOS whether an installed app claims the URL. Queried schemes must
    /// appear in `LSApplicationQueriesSchemes`.
    private static func handlerExists(for url: URL) throws -> Bool {
        UIApplication.shared.canOpenURL(url)
    }

    /// Hands the URL to the system; `false` means no app claimed it.
    private static func open(_ url: URL) async throws {
        guard await UIApplication.shared.open(url) else { throw PluginError.unavailable }
    }
    #elseif os(macOS)
    /// macOS has no settings page a plugin can open on the user's behalf.
    private static func openSystemSettings() async throws {
        throw PluginError.unavailable
    }

    /// Asks Launch Services for an installed handler.
    private static func handlerExists(for url: URL) throws -> Bool {
        NSWorkspace.shared.urlForApplication(toOpen: url) != nil
    }

    /// Hands the URL to the default application.
    private static func open(_ url: URL) async throws {
        guard NSWorkspace.shared.open(url) else { throw PluginError.unavailable }
    }
    #else
    /// No Apple platform besides iOS and macOS exposes a settings deep link.
    private static func openSystemSettings() async throws {
        throw PluginError.unavailable
    }

    /// No handler discovery API is available on this platform.
    private static func handlerExists(for url: URL) throws -> Bool {
        throw PluginError.unavailable
    }

    /// No launcher is available on this platform.
    private static func open(_ url: URL) async throws {
        throw PluginError.unavailable
    }
    #endif
}
