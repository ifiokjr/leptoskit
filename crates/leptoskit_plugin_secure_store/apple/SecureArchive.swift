import Foundation
import LeptosKitBridge

/// Names, framing and values share one credential-sized budget.
public struct SecureStoreQuota: Sendable {
    public let maxBytes: Int
    public let maxEntries: Int

    public init(maxBytes: Int = 2560, maxEntries: Int = 64) {
        self.maxBytes = maxBytes
        self.maxEntries = maxEntries
    }

    func validate() throws {
        guard (4...2560).contains(maxBytes), (0...64).contains(maxEntries) else {
            throw PluginError.invalidArgument
        }
    }
}

struct SecureArchive {
    struct Entry {
        let service: String
        let key: String
        let value: Data
    }

    private var entries: [Entry] = []

    init() {}

    init(data: Data) throws {
        guard data.count <= 2560 else { throw PluginError.tooLarge }
        try data.withUnsafeBytes { bytes in
            var offset = 0
            func count() throws -> Int {
                guard bytes.count - offset >= 4 else { throw PluginError.native }
                defer { offset += 4 }
                return Int(UInt32(littleEndian: bytes.loadUnaligned(fromByteOffset: offset, as: UInt32.self)))
            }
            let length = try count()
            guard length <= 64 else { throw PluginError.native }
            for _ in 0..<length {
                let start = offset
                for _ in 0..<3 {
                    let size = try count()
                    guard size <= bytes.count - offset else { throw PluginError.native }
                    offset += size
                }
                let request = try SecureRequest(
                    method: 1, bytes: UnsafeRawBufferPointer(rebasing: bytes[start..<offset])
                )
                guard value(service: request.service, key: request.key) == nil,
                      let secret = request.secret else { throw PluginError.native }
                entries.append(Entry(service: request.service, key: request.key, value: secret))
            }
            guard offset == bytes.count else { throw PluginError.native }
        }
    }

    func value(service: String, key: String) -> Data? {
        entries.first { $0.service == service && $0.key == key }?.value
    }

    mutating func remove(service: String, key: String) {
        entries.removeAll { $0.service == service && $0.key == key }
    }

    mutating func set(service: String, key: String, value: Data) {
        remove(service: service, key: key)
        entries.append(Entry(service: service, key: key, value: value))
    }

    func encode(quota: SecureStoreQuota) throws -> Data {
        try quota.validate()
        guard entries.count <= quota.maxEntries else { throw PluginError.tooLarge }
        var data = Data()
        func count(_ value: Int) {
            var value = UInt32(value).littleEndian
            withUnsafeBytes(of: &value) { data.append(contentsOf: $0) }
        }
        func field(_ value: Data) {
            count(value.count)
            data.append(value)
        }
        count(entries.count)
        for entry in entries {
            field(Data(entry.service.utf8))
            field(Data(entry.key.utf8))
            field(entry.value)
        }
        guard data.count <= quota.maxBytes else { throw PluginError.tooLarge }
        return data
    }
}
