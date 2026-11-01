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
        let process = ProcessInfo.processInfo

        #if os(iOS)
        let device = UIDevice.current
        let model = device.model
        let operatingSystem = device.systemName
        let version = device.systemVersion
        #else
        let model = ""

        let operatingSystem = "macOS"
        let systemVersion = process.operatingSystemVersion
        let version = "\(systemVersion.majorVersion).\(systemVersion.minorVersion).\(systemVersion.patchVersion)"
        #endif
        let memory = process.physicalMemory
        var result = Data(capacity: 128)

        for field in [1, Self.kind, Self.simulator, UInt32(truncatingIfNeeded: memory), UInt32(memory >> 32)] {
            var little = field.littleEndian
            withUnsafeBytes(of: &little) { result.append(contentsOf: $0) }
        }

        for text in ["Apple", "Apple", model, operatingSystem, version, Self.architecture] {
            let bytes = Data(text.utf8)

            guard bytes.count <= 4096 else { throw PluginError.tooLarge }
            var length = UInt32(bytes.count).littleEndian
            withUnsafeBytes(of: &length) { result.append(contentsOf: $0) }
            result.append(bytes)
        }

        return result
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
