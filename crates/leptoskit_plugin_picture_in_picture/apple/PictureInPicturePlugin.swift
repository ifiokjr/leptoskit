import AVKit
import Foundation
import LeptosKitBridge
#if os(iOS)
import UIKit
#endif

/// Reports system picture-in-picture support. Entering PiP needs an attached
/// `AVPlayerLayer` / player controller, which this release does not own yet.
/// iOS has no draw-over-apps overlay equivalent to Android's
/// `SYSTEM_ALERT_WINDOW`.
@MainActor
public final class PictureInPicturePlugin: NativePlugin {
    public let name = "picture-in-picture"
    public let methods: [UInt32] = [0, 1, 2, 3, 4, 5]

    private var closed = false

    public init() {}

    public func invoke(method: UInt32, payload: NativeBuffer) async throws -> Data {
        guard !closed else { throw PluginError.cancelled }

        guard method <= 5 else { throw PluginError.unavailable }

        guard payload.count == 0 else { throw PluginError.invalidArgument }

        switch method {
        case 0:
            return capabilities()
        case 1:
            return snapshot()
        case 4:
            return encode(permission: .unsupported)
        case 2, 3, 5:
            // Entering PiP without a player, system overlays, and overlay
            // settings are unavailable on this platform in this release.
            throw PluginError.unavailable
        default:
            throw PluginError.unavailable
        }
    }

    public func close() {
        closed = true
    }

    private func capabilities() -> Data {
        #if os(iOS)
        let supported = AVPictureInPictureController.isPictureInPictureSupported
        return words(Self.wireVersion, supported ? 1 : 0, 0)
        #else
        return words(Self.wireVersion, 0, 0)
        #endif
    }

    private func snapshot() -> Data {
        words(Self.wireVersion, Self.sessionNone)
    }

    private enum OverlayPermission: UInt32 {
        case unsupported = 0
    }

    private func encode(permission: OverlayPermission) -> Data {
        var value = permission.rawValue.littleEndian

        return withUnsafeBytes(of: &value) { Data($0) }
    }

    private func words(_ values: UInt32...) -> Data {
        var data = Data(capacity: values.count * 4)

        for value in values {
            var little = value.littleEndian

            withUnsafeBytes(of: &little) { data.append(contentsOf: $0) }
        }

        return data
    }

    private static let wireVersion: UInt32 = 1
    private static let sessionNone: UInt32 = 0
}
