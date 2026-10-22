import Foundation
import LeptosKitBridge

/// Apple exposes no public ambient-light sensor API for iOS apps.
@MainActor
public final class LightSensorPlugin: NativePlugin {
    public let name = "light_sensor"
    public let methods: [UInt32] = [0, 1]
    private var closed = false
    public init() {}
    public func invoke(method: UInt32, payload: NativeBuffer) async throws -> Data {
        if closed { throw PluginError.cancelled }
        if method == 0 {
            guard payload.count == 0 else { throw PluginError.invalidArgument }
            return Data([0, 0, 0, 0])
        }
        if method == 1 {
            guard payload.count == 8 else { throw PluginError.invalidArgument }
            let bytes = payload.withUnsafeBytes { Array($0) }
            func word(_ index: Int) -> UInt32 {
                (0..<4).reduce(0) { $0 | UInt32(bytes[index + $1]) << (8 * $1) }
            }
            guard (5...60_000).contains(word(0)), (1...120_000).contains(word(4)) else {
                throw PluginError.invalidArgument
            }
        }
        throw PluginError.unavailable
    }
    public func close() { closed = true }
}
