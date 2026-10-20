import Foundation
import LeptosKitBridge
import LocalAuthentication
import Security

@MainActor
public final class SecureStorePlugin: NativePlugin {
    public let name = "secure-store"
    public let methods: [UInt32] = [0, 1, 2]
    private var operation: Task<Data, Error>?
    private var closed = false

    public init() {}

    public func close() {
        closed = true
        operation?.cancel()
    }

    public func invoke(method: UInt32, payload: NativeBuffer) async throws -> Data {
        guard !closed else { throw PluginError.cancelled }
        guard operation == nil else { throw PluginError.busy }
        guard methods.contains(method) else { throw PluginError.unavailable }
        try Task.checkCancellation()
        guard let application = Bundle.main.bundleIdentifier else { throw PluginError.unavailable }
        let request = try payload.withUnsafeBytes { bytes in
            try SecureRequest(method: method, bytes: bytes)
        }
        // Keychain IPC can block. Only the bounded, owned request crosses actors.
        let operation = Task.detached {
            try Task.checkCancellation()
            return try Self.execute(request, application: application)
        }
        self.operation = operation
        // Keep the slot occupied until IPC finishes, even if the caller cancels.
        defer { self.operation = nil }
        return try await withTaskCancellationHandler {
            try await operation.value
        } onCancel: {
            operation.cancel()
        }
    }

    nonisolated private static func execute(_ request: SecureRequest, application: String) throws -> Data {
        let context = LAContext()
        context.interactionNotAllowed = true
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: "leptoskit.secure-store:\(application):\(request.service)",
            kSecAttrAccount as String: request.key,
            kSecAttrSynchronizable as String: false,
            kSecUseAuthenticationContext as String: context
        ]
        switch request.method {
        case 0:
            var read = query
            read[kSecReturnData as String] = true
            read[kSecMatchLimit as String] = kSecMatchLimitOne
            var result: CFTypeRef?
            let status = SecItemCopyMatching(read as CFDictionary, &result)
            if status == errSecItemNotFound { return Data([0, 0, 0, 0]) }
            try check(status)
            guard let value = result as? Data, value.count <= 2048 else { throw PluginError.native }
            var output = Data([1, 0, 0, 0])
            output.append(value)
            return output
        case 1:
            guard let value = request.secret else { throw PluginError.invalidArgument }
            let attributes: [String: Any] = [
                kSecValueData as String: value,
                kSecAttrAccessible as String: kSecAttrAccessibleWhenUnlockedThisDeviceOnly
            ]
            let status = SecItemUpdate(query as CFDictionary, attributes as CFDictionary)
            if status == errSecItemNotFound {
                var insert = query
                insert.merge(attributes) { _, updated in updated }
                let added = SecItemAdd(insert as CFDictionary, nil)
                // Another process may have inserted this entry between update and add.
                if added == errSecDuplicateItem {
                    try check(SecItemUpdate(query as CFDictionary, attributes as CFDictionary))
                } else {
                    try check(added)
                }
            } else {
                try check(status)
            }
            return Data()
        case 2:
            let status = SecItemDelete(query as CFDictionary)
            if status != errSecItemNotFound { try check(status) }
            return Data()
        default:
            throw PluginError.unavailable
        }
    }

    nonisolated private static func check(_ status: OSStatus) throws {
        switch status {
        case errSecSuccess: return
        case errSecAuthFailed, errSecInteractionNotAllowed, errSecMissingEntitlement:
            throw PluginError.permissionDenied
        case errSecNotAvailable: throw PluginError.unavailable
        default: throw PluginError.native
        }
    }
}

struct SecureRequest: Sendable {
    let method: UInt32
    let service: String
    let key: String
    let secret: Data?

    init(method: UInt32, bytes: UnsafeRawBufferPointer) throws {
        var offset = 0
        func field(limit: Int) throws -> UnsafeRawBufferPointer {
            guard bytes.count - offset >= 4 else { throw PluginError.invalidArgument }
            let count = Int(UInt32(littleEndian: bytes.loadUnaligned(fromByteOffset: offset, as: UInt32.self)))
            offset += 4
            if limit == 2048 && count > limit { throw PluginError.tooLarge }
            guard count <= limit, count <= bytes.count - offset else { throw PluginError.invalidArgument }
            let result = UnsafeRawBufferPointer(rebasing: bytes[offset..<(offset + count)])
            offset += count
            return result
        }
        func name() throws -> String {
            let value = try field(limit: 128)
            guard !value.isEmpty, value.allSatisfy({ byte in
                (65...90).contains(byte) || (97...122).contains(byte) || (48...57).contains(byte)
                    || byte == 45 || byte == 46 || byte == 95
            }) else { throw PluginError.invalidArgument }
            guard let name = String(bytes: value, encoding: .ascii) else { throw PluginError.invalidArgument }
            return name
        }
        self.method = method
        self.service = try name()
        self.key = try name()
        self.secret = method == 1 ? Data(try field(limit: 2048)) : nil
        guard offset == bytes.count else { throw PluginError.invalidArgument }
    }
}
