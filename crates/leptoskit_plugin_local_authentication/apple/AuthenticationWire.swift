import Foundation
import LeptosKitBridge

struct AuthenticationRequest {
    let policy: UInt32
    let reason: String
    let cancelLabel: String

    init(payload: NativeBuffer) throws {
        self = try payload.withUnsafeBytes { bytes in
            var input = AuthenticationReader(bytes: bytes)
            guard try input.integer() == 1 else { throw PluginError.invalidArgument }
            let policy = try input.integer()
            guard policy <= 1 else { throw PluginError.invalidArgument }
            let reason = try input.text(limit: 256)
            let cancelLabel = try input.text(limit: 64)

            guard input.finished else { throw PluginError.invalidArgument }

            return AuthenticationRequest(policy: policy, reason: reason, cancelLabel: cancelLabel)
        }
    }

    private init(policy: UInt32, reason: String, cancelLabel: String) {
        self.policy = policy
        self.reason = reason
        self.cancelLabel = cancelLabel
    }
}

private struct AuthenticationReader {
    let bytes: UnsafeRawBufferPointer
    var offset = 0
    var finished: Bool { offset == bytes.count }

    mutating func integer() throws -> UInt32 {
        guard bytes.count - offset >= 4 else { throw PluginError.invalidArgument }
        defer { offset += 4 }

        return bytes.loadUnaligned(fromByteOffset: offset, as: UInt32.self).littleEndian
    }

    mutating func text(limit: Int) throws -> String {
        let count = Int(try integer())

        guard count <= limit else { throw PluginError.tooLarge }

        guard count <= bytes.count - offset,
              let value = String(bytes: bytes[offset..<(offset + count)], encoding: .utf8),
              !value.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
              !value.contains("\0") else { throw PluginError.invalidArgument }
        offset += count

        return value
    }
}

func authenticationFields(_ fields: [UInt32]) -> Data {
    var output = Data(capacity: fields.count * 4)

    for field in fields {
        var little = field.littleEndian
        withUnsafeBytes(of: &little) { output.append(contentsOf: $0) }
    }

    return output
}
