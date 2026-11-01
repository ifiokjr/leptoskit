import Foundation
import LeptosKitBridge
import LocalAuthentication

/// Byte limits the Rust crate enforces on the prompt text.
private enum Limits {
    /// Longest accepted reason, in UTF-8 bytes.
    static let reasonBytes = 256
    /// Longest accepted cancel label, in UTF-8 bytes.
    static let cancelBytes = 64
}

/// Readiness codes the capability reply carries.
enum Readiness {
    static let ready: UInt32 = 0
    static let unavailable: UInt32 = 1
    static let notEnrolled: UInt32 = 2
    static let lockedOut: UInt32 = 3
    static let credentialNotSet: UInt32 = 4
    static let unknown: UInt32 = 7
}

/// Terminal outcomes the authentication reply carries.
enum Outcome {
    static let failed: UInt32 = 1
    static let userCancelled: UInt32 = 2
    static let systemCancelled: UInt32 = 3
    static let appCancelled: UInt32 = 4
    static let lockedOut: UInt32 = 5
    static let notEnrolled: UInt32 = 6
    static let unavailable: UInt32 = 7
    static let credentialNotSet: UInt32 = 8
}

/// Sensor bits the capability reply carries. Android also reports iris hardware.
enum Modality {
    static let none: UInt32 = 0
    static let fingerprint: UInt32 = 1
    static let face: UInt32 = 2
    static let optic: UInt32 = 8
}

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
            let reason = try input.text(limit: Limits.reasonBytes)
            let cancelLabel = try input.text(limit: Limits.cancelBytes)

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

/// Decodes the policy a capability query asked about.
func decodePolicy(_ payload: NativeBuffer) throws -> UInt32 {
    try payload.withUnsafeBytes { bytes -> UInt32 in

        guard bytes.count == 4 else { throw PluginError.invalidArgument }
        let value = bytes.loadUnaligned(as: UInt32.self).littleEndian

        guard value <= 1 else { throw PluginError.invalidArgument }

        return value
    }
}

/// The sensor bits the OS reports for its biometric hardware.
func modality(_ type: LABiometryType) -> UInt32 {
    switch type {
    case .touchID: Modality.fingerprint
    case .faceID: Modality.face
    case .opticID: Modality.optic
    default: Modality.none
    }
}

/// The readiness code that explains why the policy cannot be evaluated.
func readiness(for error: NSError?) -> UInt32 {
    guard error?.domain == LAError.errorDomain, let code = error?.code else { return Readiness.unknown }

    switch code {
    case LAError.biometryNotAvailable.rawValue: return Readiness.unavailable
    case LAError.biometryNotEnrolled.rawValue: return Readiness.notEnrolled
    case LAError.biometryLockout.rawValue: return Readiness.lockedOut
    case LAError.passcodeNotSet.rawValue: return Readiness.credentialNotSet
    default: return Readiness.unknown
    }
}

/// The terminal outcome for one OS authentication failure.
func outcome(for error: NSError?) throws -> UInt32 {
    guard let error, error.domain == LAError.errorDomain else { throw PluginError.native }

    return try switch error.code {
    case LAError.authenticationFailed.rawValue: Outcome.failed
    case LAError.userCancel.rawValue, LAError.userFallback.rawValue: Outcome.userCancelled
    case LAError.systemCancel.rawValue: Outcome.systemCancelled
    case LAError.appCancel.rawValue: Outcome.appCancelled
    case LAError.biometryLockout.rawValue: Outcome.lockedOut
    case LAError.biometryNotEnrolled.rawValue: Outcome.notEnrolled
    case LAError.biometryNotAvailable.rawValue, LAError.notInteractive.rawValue: Outcome.unavailable
    case LAError.passcodeNotSet.rawValue: Outcome.credentialNotSet
    default: throw PluginError.native
    }
}
