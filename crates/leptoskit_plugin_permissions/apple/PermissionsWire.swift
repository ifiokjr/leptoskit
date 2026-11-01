import AVFoundation
import Foundation
import LeptosKitBridge

/// One permission the Rust side sent, with what AVFoundation needs to answer it.
///
/// AVFoundation has no permission enum of its own: a capability is a media type,
/// and the usage description that must accompany it is a separate Info.plist key.
struct Kind {
    /// The media type this capability is authorized for.
    let media: AVMediaType
    /// The Info.plist key Apple requires before it will show the prompt.
    let usageDescription: String

    /// Decodes the payload's single little-endian kind.
    init(_ payload: NativeBuffer) throws {
        let raw = try payload.withUnsafeBytes { bytes -> UInt32 in

            guard bytes.count == 4 else { throw PluginError.invalidArgument }

            return UInt32(littleEndian: bytes.loadUnaligned(as: UInt32.self))
        }

        switch raw {
        case 0:
            media = .video
            usageDescription = "NSCameraUsageDescription"
        case 1:
            media = .audio
            usageDescription = "NSMicrophoneUsageDescription"
        default:
            throw PluginError.invalidArgument
        }
    }
}
