import AVFoundation
import Foundation
import LeptosKitBridge

@MainActor
public final class PermissionsPlugin: NativePlugin {
    public let name = "permissions"
    public let methods: [UInt32] = [0, 1]
    /// Media types with a system prompt in flight; each type is independent.
    private var requesting: Set<AVMediaType> = []
    private var closed = false

    public init() {}

    public func close() { closed = true }

    public func invoke(method: UInt32, payload: NativeBuffer) async throws -> Data {
        guard !closed else { throw PluginError.cancelled }

        guard methods.contains(method) else { throw PluginError.unavailable }
        let kind = try payload.withUnsafeBytes { bytes -> UInt32 in

            guard bytes.count == 4 else { throw PluginError.invalidArgument }

            return UInt32(littleEndian: bytes.loadUnaligned(as: UInt32.self))
        }
        let media: AVMediaType
        let description: String
        switch kind {
        case 0:
            media = .video
            description = "NSCameraUsageDescription"
        case 1:
            media = .audio
            description = "NSMicrophoneUsageDescription"
        default: throw PluginError.invalidArgument
        }

        if method == 1 && AVCaptureDevice.authorizationStatus(for: media) == .notDetermined {
            guard !requesting.contains(media) else { throw PluginError.busy }

            guard let purpose = Bundle.main.object(forInfoDictionaryKey: description) as? String,
                  !purpose.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
                // Apple terminates apps requesting access without this declaration.
                throw PluginError.invalidArgument
            }
            try Task.checkCancellation()
            requesting.insert(media)
            defer { requesting.remove(media) }
            _ = await AVCaptureDevice.requestAccess(for: media)

            try Task.checkCancellation()

            guard !closed else { throw PluginError.cancelled }
        }

        return response(for: media)
    }

    private func response(for media: AVMediaType) -> Data {
        let status: UInt32
        let canPrompt: UInt32
        switch AVCaptureDevice.authorizationStatus(for: media) {
        case .notDetermined: (status, canPrompt) = (1, 2)
        case .denied: (status, canPrompt) = (2, 1)
        case .authorized: (status, canPrompt) = (3, 1)
        case .restricted: (status, canPrompt) = (4, 1)
        @unknown default: (status, canPrompt) = (0, 0)
        }
        var result = Data()

        for var value in [status.littleEndian, canPrompt.littleEndian] {
            withUnsafeBytes(of: &value) { result.append(contentsOf: $0) }
        }

        return result
    }
}
