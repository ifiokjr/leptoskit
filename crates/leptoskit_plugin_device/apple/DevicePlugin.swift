import Foundation
import LeptosKitBridge
#if os(iOS)
import UIKit
#endif

@MainActor
public final class DevicePlugin: NativePlugin {
    public let name = "device"
    public let methods: [UInt32] = [0]

    public init() {}

    public func invoke(method: UInt32, payload: NativeBuffer) async throws -> Data {
        guard method == 0 else { throw PluginError.unavailable }
        guard payload.count == 0 else { throw PluginError.invalidArgument }

        return try encode()
    }

    /// Packs the device's kind, memory, and identifying strings.
    private func encode() throws -> Data {
        let process = ProcessInfo.processInfo
        let platform = Self.platform
        let memory = process.physicalMemory
        var result = Data(capacity: 128)

        for field in [1, Self.kind, Self.simulator, UInt32(truncatingIfNeeded: memory), UInt32(memory >> 32)] {
            Self.write(&result, field)
        }

        for text in ["Apple", "Apple", platform.model, platform.system, platform.version, Self.architecture] {
            try Self.write(&result, text)
        }

        return result
    }

    /// The model and OS this build reports.
    private struct Platform {
        let model: String
        let system: String
        let version: String
    }

    /// UIKit's device on iOS, and the process on macOS.
    private static var platform: Platform {
        #if os(iOS)
        let device = UIDevice.current

        return Platform(model: device.model, system: device.systemName, version: device.systemVersion)
        #else
        let system = ProcessInfo.processInfo.operatingSystemVersion
        let version = "\(system.majorVersion).\(system.minorVersion).\(system.patchVersion)"

        return Platform(model: "", system: "macOS", version: version)
        #endif
    }

    /// Appends a little-endian word.
    private static func write(_ result: inout Data, _ value: UInt32) {
        var little = value.littleEndian
        withUnsafeBytes(of: &little) { result.append(contentsOf: $0) }
    }

    /// Appends a length-prefixed UTF-8 string.
    private static func write(_ result: inout Data, _ text: String) throws {
        let bytes = Data(text.utf8)

        guard bytes.count <= 4096 else { throw PluginError.tooLarge }
        write(&result, UInt32(bytes.count))
        result.append(bytes)
    }

    private static var kind: UInt32 {
        #if os(iOS)
        switch UIDevice.current.userInterfaceIdiom {
        case .phone: 1
        case .pad: 2
        case .tv: 3
        case .mac: 4
        case .carPlay: 6
        case .vision: 7
        default: 0
        }
        #else
        4
        #endif
    }

    private static var simulator: UInt32 {
        #if targetEnvironment(simulator)
        2
        #elseif os(iOS)
        1
        #else
        0
        #endif
    }

    private static var architecture: String {
        #if arch(arm64)
        "aarch64"
        #elseif arch(x86_64)
        "x86_64"
        #else
        ""
        #endif
    }
}
