import Foundation
import LeptosKitBridge

enum ShareContent {
    case text(String)
    case url(URL)

    /// Two u32 fields frame the value.
    private static let headerBytes = 8

    /// The largest value each kind may carry.
    private static let textBytes = 65536
    private static let urlBytes = 8192

    /// Only printable ASCII may appear literally in a shared URL.
    private static let firstPrintable: UInt8 = 33
    private static let lastPrintable: UInt8 = 126
    private static let backslash: UInt8 = 92
    private static let percent: UInt8 = 37

    /// The scheme prefixes a URL must start with, in bytes.
    private static let httpsPrefix = 8
    private static let httpPrefix = 7

    /// The highest port a URL may name.
    private static let maxPort = 65535

    var item: Any {
        switch self {
        case .text(let text): return text
        case .url(let url): return url
        }
    }

    /// Decodes a framed text or URL payload, refusing unknown kinds.
    static func decode(_ bytes: UnsafeRawBufferPointer) throws -> ShareContent {
        guard bytes.count >= headerBytes else { throw PluginError.invalidArgument }

        let kind = UInt32(littleEndian: bytes.loadUnaligned(as: UInt32.self))
        let count = Int(UInt32(littleEndian: bytes.loadUnaligned(fromByteOffset: 4, as: UInt32.self)))

        guard kind <= 1, bytes.count - headerBytes == count else { throw PluginError.invalidArgument }

        guard count <= (kind == 0 ? textBytes : urlBytes) else { throw PluginError.tooLarge }

        let value = try decodeValue(bytes, count: count)

        if kind == 0 { return .text(value) }

        return .url(try webURL(value))
    }

    /// Decodes the bounded UTF-8 body, refusing an empty value or one carrying NUL.
    private static func decodeValue(_ bytes: UnsafeRawBufferPointer, count: Int) throws -> String {
        guard count > 0, let value = String(bytes: bytes.dropFirst(headerBytes), encoding: .utf8),
              !value.contains("\0") else {
            throw PluginError.invalidArgument
        }

        return value
    }

    /// Accepts an absolute http(s) URL with a visible-ASCII body and a bare host.
    private static func webURL(_ value: String) throws -> URL {
        let authority = try webAuthority(value)

        guard let url = URL(string: value), let host = url.host, !host.isEmpty else {
            throw PluginError.invalidArgument
        }

        guard !authority.contains("@"), url.user == nil, url.password == nil else {
            throw PluginError.invalidArgument
        }

        guard url.port == nil || (0...maxPort).contains(url.port ?? -1) else {
            throw PluginError.invalidArgument
        }

        try requireBody(value)

        return url
    }

    /// Reads the authority between the scheme and the first path, query, or fragment.
    private static func webAuthority(_ value: String) throws -> Substring {
        guard isVisible(value) else { throw PluginError.invalidArgument }

        guard value.hasPrefix("https://") || value.hasPrefix("http://") else {
            throw PluginError.invalidArgument
        }

        guard let authority = value.components(separatedBy: "://").dropFirst().first?
            .split(whereSeparator: { "/?#".contains($0) }).first, !authority.isEmpty else {
            throw PluginError.invalidArgument
        }

        return authority
    }

    /// Only printable ASCII may appear literally, so no control or backslash hides in the value.
    private static func isVisible(_ value: String) -> Bool {
        value.utf8.allSatisfy { (firstPrintable...lastPrintable).contains($0) && $0 != backslash }
    }

    /// Foundation accepts more spellings than the guest protocol; the body must not
    /// begin with a separator an empty authority would hide.
    private static func requireBody(_ value: String) throws {
        let prefix = value.hasPrefix("https://") ? httpsPrefix : httpPrefix
        let suffix = value.dropFirst(prefix)

        guard suffix.first != "/", suffix.first != "?", suffix.first != "#" else {
            throw PluginError.invalidArgument
        }

        try requireEscapes(Array(value.utf8))
    }

    /// Every percent sign must introduce two hexadecimal digits.
    private static func requireEscapes(_ bytes: [UInt8]) throws {
        for index in bytes.indices where bytes[index] == percent {
            guard index + 2 < bytes.count, isHex(bytes[index + 1]), isHex(bytes[index + 2]) else {
                throw PluginError.invalidArgument
            }
        }
    }

    /// Digits, upper-case hexadecimal, and lower-case hexadecimal.
    private static func isHex(_ value: UInt8) -> Bool {
        (48...57).contains(value) || (65...70).contains(value) || (97...102).contains(value)
    }
}
