import Darwin
import Foundation
import LeptosKitBridge
import LocalAuthentication
import Security

@MainActor
public final class SecureStorePlugin: NativePlugin {
    public let name = "secure-store"
    public let methods: [UInt32] = [0, 1, 2]
    /// Keychain calls run one at a time in request order, matching Android's
    /// serialized store, so concurrent guest calls queue instead of failing.
    private var tail: Task<Void, Never>?
    private var inFlight: [UUID: Task<Data, Error>] = [:]
    private var closed = false
    private let quota: SecureStoreQuota
    nonisolated private static let storeLock = NSLock()

    public init(quota: SecureStoreQuota = SecureStoreQuota()) {
        self.quota = quota
    }

    public func close() {
        closed = true

        for operation in inFlight.values { operation.cancel() }
    }

    public func invoke(method: UInt32, payload: NativeBuffer) async throws -> Data {
        guard !closed else { throw PluginError.cancelled }

        guard methods.contains(method) else { throw PluginError.unavailable }
        try Task.checkCancellation()

        guard let application = Bundle.main.bundleIdentifier else { throw PluginError.unavailable }
        let request = try payload.withUnsafeBytes { bytes in
            try SecureRequest(method: method, bytes: bytes)
        }
        // Keychain IPC can block. Only the bounded, owned request crosses actors.
        let previous = tail
        let quota = quota
        let operation = Task.detached {
            await previous?.value
            try Task.checkCancellation()

            return try Self.storeLock.withLock {
                try Self.withFileLock(application: application) {
                    try Self.execute(request, application: application, quota: quota)
                }
            }
        }
        tail = Task { _ = try? await operation.value }

        let identifier = UUID()
        inFlight[identifier] = operation
        defer { inFlight[identifier] = nil }

        return try await withTaskCancellationHandler {
            try await operation.value
        } onCancel: {
            operation.cancel()
        }
    }

    nonisolated private static func withFileLock<T>(application: String, operation: () throws -> T) throws -> T {
        let support = try FileManager.default.url(
            for: .applicationSupportDirectory, in: .userDomainMask, appropriateFor: nil, create: true
        )
        let directory = support.appendingPathComponent("leptoskit-secure-store-locks", isDirectory: true)
            .appendingPathComponent(application, isDirectory: true)
        try FileManager.default.createDirectory(
            at: directory, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700]
        )
        let descriptor = Darwin.open(
            directory.appendingPathComponent(".lock").path, O_CREAT | O_RDWR | O_NOFOLLOW | O_CLOEXEC, 0o600
        )

        guard descriptor >= 0 else { throw PluginError.native }
        defer { Darwin.close(descriptor) }

        guard flock(descriptor, LOCK_EX) == 0 else { throw PluginError.native }
        defer { _ = flock(descriptor, LOCK_UN) }

        return try operation()
    }

    nonisolated private static func execute(
        _ request: SecureRequest, application: String, quota: SecureStoreQuota
    ) throws -> Data {
        try quota.validate()
        let query = credentialQuery(service: "leptoskit.secure-store-v2:\(application)", key: "entries")
        var archive = try read(query).map { try SecureArchive(data: $0) } ?? SecureArchive()
        switch request.method {
        case 0:
            let value = try archive.value(service: request.service, key: request.key)
                ?? readLegacy(request, application: application)

            guard let value else { return Data([0, 0, 0, 0]) }

            guard value.count <= 2048 else { throw PluginError.tooLarge }
            return Data([1, 0, 0, 0]) + value
        case 1:
            guard let value = request.secret else { throw PluginError.invalidArgument }
            archive.set(service: request.service, key: request.key, value: value)
            try write(query, value: archive.encode(quota: quota))
            try deleteLegacy(request, application: application)
        case 2:
            try deleteLegacy(request, application: application)
            archive.remove(service: request.service, key: request.key)
            try write(query, value: archive.encode(quota: SecureStoreQuota()))
        default:
            throw PluginError.unavailable
        }

        return Data()
    }

    nonisolated private static func credentialQuery(service: String, key: String) -> [String: Any] {
        let context = LAContext()
        context.interactionNotAllowed = true
        return [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: key,
            kSecAttrSynchronizable as String: false,
            kSecUseAuthenticationContext as String: context
        ]
    }

    nonisolated private static func read(_ query: [String: Any]) throws -> Data? {
        var read = query
        read[kSecReturnData as String] = true
        read[kSecMatchLimit as String] = kSecMatchLimitOne
        var result: CFTypeRef?
        let status = SecItemCopyMatching(read as CFDictionary, &result)

        if status == errSecItemNotFound { return nil }
        try check(status)

        guard let value = result as? Data, value.count <= 2560 else { throw PluginError.native }

        return value
    }

    nonisolated private static func write(_ query: [String: Any], value: Data) throws {
        if value.count == 4 {
            let status = SecItemDelete(query as CFDictionary)

            if status != errSecItemNotFound { try check(status) }

            return
        }

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
    }

    nonisolated private static func legacyQuery(_ request: SecureRequest, application: String) -> [String: Any] {
        credentialQuery(service: "leptoskit.secure-store:\(application):\(request.service)", key: request.key)
    }

    nonisolated private static func readLegacy(_ request: SecureRequest, application: String) throws -> Data? {
        try read(legacyQuery(request, application: application))
    }

    nonisolated private static func deleteLegacy(_ request: SecureRequest, application: String) throws {
        let status = SecItemDelete(legacyQuery(request, application: application) as CFDictionary)

        if status != errSecItemNotFound { try check(status) }
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
