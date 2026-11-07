import Foundation
import LeptosKitBridge

/// Apple platforms have no Device Admin / Device Policy Manager contract.
/// Absence is never an inferred grant or an MDM enrollment.
@MainActor
public final class DevicePolicyPlugin: NativePlugin {
    public let name = "device-policy"
    public let methods: [UInt32] = [0, 1, 2, 3, 4, 5, 6, 7]
    private var closed = false

    public init() {}

    public func invoke(method: UInt32, payload: NativeBuffer) async throws -> Data {
        guard !closed else { throw PluginError.cancelled }
        guard methods.contains(method) else { throw PluginError.unavailable }
        _ = payload
        throw PluginError.unavailable
    }

    public func close() {
        closed = true
    }
}
