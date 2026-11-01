import Foundation
import LeptosKitBridge

@MainActor
public final class ApplicationPlugin: NativePlugin {
    public let name = "application"
    public let methods: [UInt32] = [0]

    public init() {}

    public func invoke(method: UInt32, payload: NativeBuffer) async throws -> Data {
        guard method == 0 else { throw PluginError.unavailable }
        guard payload.count == 0 else { throw PluginError.invalidArgument }
        let bundle = Bundle.main
        let fields = [
            bundle.bundleIdentifier,
            bundle.object(forInfoDictionaryKey: "CFBundleDisplayName") as? String
                ?? bundle.object(forInfoDictionaryKey: "CFBundleName") as? String,
            bundle.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String,
            bundle.object(forInfoDictionaryKey: "CFBundleVersion") as? String
        ]
        var result = Data([1, 0, 0, 0])

        for value in fields {
            let text = Data((value ?? "").utf8)

            guard text.count <= 4096 else { throw PluginError.tooLarge }
            var length = UInt32(text.count).littleEndian
            withUnsafeBytes(of: &length) { result.append(contentsOf: $0) }
            result.append(text)
        }

        return result
    }
}
