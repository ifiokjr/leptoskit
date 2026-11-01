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

        if method == 1 { try validateWindow(payload) }

        throw PluginError.unavailable
    }

    /// Checks the polling window a watch asks for, before reporting that there
    /// is no sensor to poll.
    private func validateWindow(_ payload: NativeBuffer) throws {
        guard payload.count == 8 else { throw PluginError.invalidArgument }
        let bytes = payload.withUnsafeBytes { Array($0) }
        func word(_ index: Int) -> UInt32 {
            (0..<4).reduce(0) { $0 | UInt32(bytes[index + $1]) << (8 * $1) }
        }

        guard Self.intervalRange.contains(word(0)),
              Self.windowRange.contains(word(4)) else { throw PluginError.invalidArgument }
    }

    /// The interval a watch may poll at, in milliseconds.
    private static let intervalRange: ClosedRange<UInt32> = 5...60_000

    /// The window a watch may run for, in milliseconds.
    private static let windowRange: ClosedRange<UInt32> = 1...120_000

    public func close() { closed = true }
}
