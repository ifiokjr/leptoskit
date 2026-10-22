import Foundation
import LeptosKitBridge

enum ShareContent {
    case text(String)
    case url(URL)

    var item: Any {
        switch self {
        case .text(let text): return text
        case .url(let url): return url
        }
    }

    static func decode(_ bytes: UnsafeRawBufferPointer) throws -> ShareContent {
        guard bytes.count >= 8 else { throw PluginError.invalidArgument }
        let kind = UInt32(littleEndian: bytes.loadUnaligned(as: UInt32.self))
        let count = Int(UInt32(littleEndian: bytes.loadUnaligned(fromByteOffset: 4, as: UInt32.self)))
        guard kind <= 1, bytes.count - 8 == count else { throw PluginError.invalidArgument }
        guard count <= (kind == 0 ? 65536 : 8192) else { throw PluginError.tooLarge }
        guard count > 0, let value = String(bytes: bytes.dropFirst(8), encoding: .utf8),
              !value.contains("\0") else { throw PluginError.invalidArgument }
        if kind == 0 { return .text(value) }
        return .url(try webURL(value))
    }

    private static func webURL(_ value: String) throws -> URL {
        let bytes = Array(value.utf8)
        guard bytes.allSatisfy({ (33...126).contains($0) && $0 != 92 }),
              value.hasPrefix("https://") || value.hasPrefix("http://"),
              let authority = value.components(separatedBy: "://").dropFirst().first?
                .split(whereSeparator: { "/?#".contains($0) }).first,
              !authority.isEmpty, !authority.contains("@"),
              let url = URL(string: value), let host = url.host, !host.isEmpty,
              url.user == nil, url.password == nil,
              url.port == nil || (0...65535).contains(url.port ?? -1) else {
            throw PluginError.invalidArgument
        }
        // Foundation accepts more URL spellings than the guest's strict protocol.
        let suffix = value.dropFirst(value.hasPrefix("https://") ? 8 : 7)
        guard suffix.first != "/", suffix.first != "?", suffix.first != "#" else {
            throw PluginError.invalidArgument
        }
        for index in bytes.indices where bytes[index] == 37 {
            guard index + 2 < bytes.count, isHex(bytes[index + 1]), isHex(bytes[index + 2]) else {
                throw PluginError.invalidArgument
            }
        }
        return url
    }

    private static func isHex(_ value: UInt8) -> Bool {
        (48...57).contains(value) || (65...70).contains(value) || (97...102).contains(value)
    }
}
