import CLeptosKitBridge
import Foundation
import LeptosKitBridge
@testable import LeptosKitBadge
import Testing

@MainActor
private final class BadgeSpy: BadgeService {
    var counts: [UInt32] = []
    private var current: UInt32 = 0
    func get() -> UInt32 { current }
    func set(_ count: UInt32) async throws {
        current = count
        counts.append(count)
    }
}

private func payload(_ bytes: [UInt8] = []) throws -> NativeBuffer {
    let bridge = try #require(lk_bridge_create())
    defer { lk_bridge_destroy(bridge) }
    let name = Array("test".utf8)
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

@Test
@MainActor
func getSetAndClearUseInjectedService() async throws {
    let service = BadgeSpy()
    let plugin = BadgePlugin(service: service)
    #expect(try await plugin.invoke(method: 0, payload: payload()) == Data([0, 0, 0, 0]))
    #expect(try await plugin.invoke(method: 1, payload: payload([7, 0, 0, 0])) == Data())
    #expect(service.counts == [7])
    #expect(try await plugin.invoke(method: 0, payload: payload()) == Data([7, 0, 0, 0]))
    #expect(try await plugin.invoke(method: 2, payload: payload()) == Data())
    #expect(service.counts == [7, 0])
    plugin.close()
    await #expect(throws: PluginError.cancelled) {
        try await plugin.invoke(method: 0, payload: payload())
    }
}

@Test
@MainActor
func oversizedCountsAreRejected() async throws {
    let service = BadgeSpy()
    let plugin = BadgePlugin(service: service)
    await #expect(throws: PluginError.invalidArgument) {
        try await plugin.invoke(method: 1, payload: payload([64, 66, 15, 0])) // 1_000_000
    }
    #expect(service.counts.isEmpty)
}
