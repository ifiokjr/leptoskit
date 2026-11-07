import CLeptosKitBridge
import Foundation
import LeptosKitBridge
import LeptosKitNetwork
import Testing

/// Allocate a payload through the real C ABI; its bytes outlive the bridge.
@MainActor
private func payload(_ data: Data = Data()) throws -> NativeBuffer {
    let bridge = try #require(lk_bridge_create())
    defer { lk_bridge_destroy(bridge) }
    let name = Array("network".utf8)
    let methods: [UInt32] = [0]
    let plugin = name.withUnsafeBufferPointer { name in
        methods.withUnsafeBufferPointer { methods in
            lk_bridge_register(bridge, name.baseAddress, name.count, methods.baseAddress, methods.count)
        }
    }
    #expect(plugin == 0)
    #expect(lk_bridge_grant(bridge, 0, 0, true) == 0)
    let call = data.withUnsafeBytes {
        lk_bridge_submit(bridge, 0, 0, $0.baseAddress?.assumingMemoryBound(to: UInt8.self), $0.count)
    }
    #expect(call > 0)
    var request = lk_request()
    #expect(lk_bridge_next_request(bridge, &request) == 1)

    return NativeBuffer(taking: request.payload)
}

/// Lets queued main-actor work, such as a newly started request, run.
@MainActor
private func settle() async {
    for _ in 0..<20 { await Task.yield() }
}

@Test
@MainActor
func snapshotsAnswerWhileAChangeIsAwaited() async throws {
    let plugin = NetworkPlugin()
    let current = try await plugin.invoke(method: 0, payload: payload())
    #expect(current.count == 32)

    // Waiting for a change from the current reading keeps the request open.
    let change = Task { try await plugin.invoke(method: 1, payload: payload(current)) }
    await settle()

    // A snapshot has its own monitor, so it is not refused while the wait runs.
    #expect(try await plugin.invoke(method: 0, payload: payload()).count == 32)
    await #expect(throws: PluginError.busy) {
        try await plugin.invoke(method: 1, payload: payload(current))
    }

    change.cancel()
    await #expect(throws: (any Error).self) { try await change.value }
    await settle()

    // The cancelled wait released its slot.
    let replacement = Task { try await plugin.invoke(method: 1, payload: payload(current)) }
    await settle()
    plugin.close()
    await #expect(throws: PluginError.cancelled) { try await replacement.value }
    await #expect(throws: PluginError.cancelled) { try await plugin.invoke(method: 0, payload: payload()) }
}

@Test
@MainActor
func malformedRequestsAreRefusedBeforeAnyMonitorStarts() async throws {
    let plugin = NetworkPlugin()
    await #expect(throws: PluginError.invalidArgument) {
        try await plugin.invoke(method: 0, payload: payload(Data([0])))
    }
    await #expect(throws: PluginError.invalidArgument) {
        try await plugin.invoke(method: 1, payload: payload(Data(repeating: 0, count: 31)))
    }
    // A connection code past the last one the wire defines.
    var invalid = Data(repeating: 0, count: 32)
    invalid[0] = 8
    await #expect(throws: PluginError.invalidArgument) { try await plugin.invoke(method: 1, payload: payload(invalid)) }
    await #expect(throws: PluginError.unavailable) { try await plugin.invoke(method: 7, payload: payload()) }
}
