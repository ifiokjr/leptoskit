import Foundation
import LeptosKitBridge

/// The path every return address has; the server checks the same one.
let webAuthReturnPath = "/auth/native/return"

/// The private-use return address for this app: its bundle ID, lowercased,
/// as a scheme (RFC 8252 §7.1). Nil for an ID that can't be one.
func webAuthReturnAddress(bundleIdentifier: String) -> String? {
    let scheme = bundleIdentifier.lowercased()
    let segments = scheme.split(separator: ".", omittingEmptySubsequences: false)
    guard segments.count >= 2, segments.allSatisfy({ segment in
        guard let first = segment.unicodeScalars.first, ("a"..."z").contains(first) else { return false }
        return segment.unicodeScalars.allSatisfy { ("a"..."z").contains($0) || ("0"..."9").contains($0) }
    }) else { return nil }
    return "\(scheme):\(webAuthReturnPath)"
}

/// `BEGIN`'s answer: the session's little-endian ID, then its return address.
func encodeWebAuthSession(id: UInt32, returnTo: String) -> Data {
    withUnsafeBytes(of: id.littleEndian) { Data($0) } + Data(returnTo.utf8)
}

/// An `AUTHENTICATE` request: the session, and the provider's page.
struct WebAuthRequest {
    let session: UInt32
    let url: URL

    init(payload: NativeBuffer) throws {
        let bytes = payload.withUnsafeBytes { Data($0) }
        guard bytes.count > 4, bytes.count <= 4 + 8192 else { throw PluginError.invalidArgument }
        session = bytes.prefix(4).enumerated().reduce(UInt32(0)) { value, byte in
            value | UInt32(byte.element) << (8 * UInt32(byte.offset))
        }
        guard let text = String(data: bytes.dropFirst(4), encoding: .utf8) else {
            throw PluginError.invalidArgument
        }
        url = try Self.providerPage(text)
    }

    /// `https`, or `http` on this device for development; no credentials.
    static func providerPage(_ text: String) throws -> URL {
        guard text.utf8.allSatisfy({ $0 >= 33 && $0 <= 126 }),
              let components = URLComponents(string: text),
              let scheme = components.scheme?.lowercased(),
              let host = components.host, !host.isEmpty,
              components.user == nil, components.password == nil,
              let url = components.url else { throw PluginError.invalidArgument }
        let local = host == "localhost" || host == "127.0.0.1" || host == "::1" || host == "[::1]"
        guard scheme == "https" || (scheme == "http" && local) else { throw PluginError.invalidArgument }
        return url
    }
}

/// Whether the browser came back to `returnTo` itself, with only a query.
func webAuthCameBack(_ url: URL, to returnTo: String) -> Bool {
    guard let back = URLComponents(url: url, resolvingAgainstBaseURL: false),
          let expected = URLComponents(string: returnTo) else { return false }
    return back.scheme?.lowercased() == expected.scheme?.lowercased()
        && back.host == expected.host && back.port == expected.port
        && back.path == expected.path && back.fragment == nil
        && back.user == nil && back.password == nil
}
