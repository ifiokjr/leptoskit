import CLeptosKitBridge
import Foundation
import LeptosKitBridge
@testable import LeptosKitPedometer
import Testing

@MainActor
private func payload(_ bytes: [UInt8]) throws -> NativeBuffer {
    let bridge = try #require(lk_bridge_create())
    defer { lk_bridge_destroy(bridge) }
    let name = Array("sensor-test".utf8)
    let methods: [UInt32] = [0]
    let plugin = name.withUnsafeBufferPointer { name in
        methods.withUnsafeBufferPointer { methods in
            lk_bridge_register(bridge, name.baseAddress, name.count, methods.baseAddress, methods.count)
        }
    }
    #expect(plugin == 0)
    #expect(lk_bridge_grant(bridge, 0, 0, true) == 0)
    let call = bytes.withUnsafeBufferPointer { lk_bridge_submit(bridge, 0, 0, $0.baseAddress, $0.count) }
    #expect(call > 0)
    var request = lk_request()
    #expect(lk_bridge_next_request(bridge, &request) == 1)

    return NativeBuffer(taking: request.payload)
}

@Test @MainActor
func unavailableAndClosedAreExplicit() async throws {
    let plugin = PedometerPlugin()

    #if os(macOS)
    #expect(try await plugin.invoke(method: 0, payload: payload([])) == Data([0, 0, 0, 0]))
    #endif
    await #expect(throws: PluginError.invalidArgument) { try await plugin.invoke(method: 0, payload: payload([1])) }
    plugin.close()
    await #expect(throws: PluginError.cancelled) { try await plugin.invoke(method: 0, payload: payload([])) }
}
@Test func intervalEncodingRejectsInventedCountsAndBadClocks() throws {
    let sample = try StepSample(start: 100, end: 200, steps: 13).encode()
    #expect(sample.count == 36)
    #expect(Array(sample[12..<20]) == [13, 0, 0, 0, 0, 0, 0, 0])
    #expect(throws: PluginError.native) { try StepSample(start: 200, end: 100, steps: 1).encode() }
    #expect(throws: PluginError.native) { try StepSample(start: 100, end: 200, steps: -1).encode() }
}
