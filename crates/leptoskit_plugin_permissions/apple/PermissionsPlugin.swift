import AVFoundation
import Foundation
import LeptosKitBridge

@MainActor
public final class PermissionsPlugin: NativePlugin {
    public let name = "permissions"
    public let methods: [UInt32] = [0, 1]
    /// The method that may display a prompt; a query only reads the state.
    private static let request: UInt32 = 1
    /// Two u32 fields: state and can-prompt.
    private static let responseBytes = 8
    /// Media types with a system prompt in flight; each type is independent.
    private var requesting: Set<AVMediaType> = []
    private var closed = false

    public init() {}

    public func close() { closed = true }

    /// Answers one call: a query only reads the current state, while a request may
    /// prompt first when the system has not asked yet.
    public func invoke(method: UInt32, payload: NativeBuffer) async throws -> Data {
        guard !closed else { throw PluginError.cancelled }

        guard methods.contains(method) else { throw PluginError.unavailable }
        let kind = try Kind(payload)

        if method == Self.request, AVCaptureDevice.authorizationStatus(for: kind.media) == .notDetermined {
            try await prompt(for: kind)
        }

        return response(for: kind.media)
    }

    /// Reports AVFoundation's authorization state as the two u32 fields.
    ///
    /// `restricted` is reported separately because a parent or profile, not the
    /// user, made that choice; an unreadable status stays `unknown`.
    private func response(for media: AVMediaType) -> Data {
        let (status, canPrompt) = authorization(for: media)
        var output = Data(capacity: Self.responseBytes)
        var littleStatus = status.littleEndian
        var littlePrompt = canPrompt.littleEndian
        withUnsafeBytes(of: &littleStatus) { output.append(contentsOf: $0) }
        withUnsafeBytes(of: &littlePrompt) { output.append(contentsOf: $0) }

        return output
    }

    /// The state and can-prompt values AVFoundation's status maps onto.
    private func authorization(for media: AVMediaType) -> (status: UInt32, canPrompt: UInt32) {
        switch AVCaptureDevice.authorizationStatus(for: media) {
        case .notDetermined: return (1, 2)
        case .denied: return (2, 1)
        case .authorized: return (3, 1)
        case .restricted: return (4, 1)
        @unknown default: return (0, 0)
        }
    }

    /// Shows the system prompt once per media type; a second request reports busy.
    private func prompt(for kind: Kind) async throws {
        guard !requesting.contains(kind.media) else { throw PluginError.busy }

        try checkUsageDescription(for: kind)
        try Task.checkCancellation()
        requesting.insert(kind.media)
        defer { requesting.remove(kind.media) }
        _ = await AVCaptureDevice.requestAccess(for: kind.media)

        try Task.checkCancellation()

        guard !closed else { throw PluginError.cancelled }
    }

    /// Apple terminates apps requesting access without this declaration.
    private func checkUsageDescription(for kind: Kind) throws {
        guard let purpose = Bundle.main.object(forInfoDictionaryKey: kind.usageDescription) as? String,
              !purpose.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            throw PluginError.invalidArgument
        }
    }
}
