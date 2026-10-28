import Foundation
import LeptosKitBridge

/// Echo capability with a distinct identity from the Rust session service.
@MainActor
public final class PlatformPlugin: NativePlugin {
    public let name = "fixture-platform"
    public let methods: [UInt32] = [1]
    private let context: NativePluginContext

    public init(context: NativePluginContext) {
        self.context = context
    }

    public func invoke(method: UInt32, payload: NativeBuffer) async throws -> Data {
        guard method == 1 else { throw PluginError.invalidArgument }

        return payload.withUnsafeBytes { Data($0) }
    }
}
