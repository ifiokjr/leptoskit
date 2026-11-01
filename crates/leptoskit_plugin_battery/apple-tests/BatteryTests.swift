import CLeptosKitBridge
import Foundation
import LeptosKitBattery
import LeptosKitBridge
import Testing

/// Allocate a payload through the real C ABI; its bytes outlive the bridge.
@MainActor
private func payload(_ data: Data = Data()) throws -> NativeBuffer {
    let bridge = try #require(lk_bridge_create())
    defer { lk_bridge_destroy(bridge) }
    let name = Array("battery".utf8)
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

@Test
@MainActor
func batteryValidatesPayloadsAndCloseIsTerminal() async throws {
    let plugin = BatteryPlugin()
    let snapshot = try await plugin.invoke(method: 0, payload: payload())
    #expect(snapshot.count == 16)
    await #expect(throws: PluginError.invalidArgument) {
        try await plugin.invoke(method: 1, payload: payload(Data(repeating: 0, count: 15)))
    }
    var invalid = snapshot
    invalid[0] = 3
    let invalidSnapshot = invalid
    await #expect(throws: PluginError.invalidArgument) {
        try await plugin.invoke(method: 1, payload: payload(invalidSnapshot))
    }
    plugin.close()
    plugin.close()
    await #expect(throws: PluginError.cancelled) {
        try await plugin.invoke(method: 0, payload: payload())
    }
}

@Test
@MainActor
func batteryCancellationAndCloseReleaseWaiters() async throws {
    let plugin = BatteryPlugin()
    defer { plugin.close() }

    for _ in 0..<3 {
        let snapshot = try await plugin.invoke(method: 0, payload: payload())
        let pending = Task { @MainActor in
            try await plugin.invoke(method: 1, payload: payload(snapshot))
        }
        // Let the waiter install its OS observers before cancelling. The real
        // battery may change in this interval, so either completion is valid.
        try await Task.sleep(for: .milliseconds(10))
        pending.cancel()
        switch await pending.result {
        case .success(let value): #expect(value != snapshot)
        case .failure(let error): #expect(error is CancellationError)
        }
    }

    let snapshot = try await plugin.invoke(method: 0, payload: payload())
    let pending = Task { @MainActor in try await plugin.invoke(method: 1, payload: payload(snapshot)) }
    try await Task.sleep(for: .milliseconds(10))
    plugin.close()
    switch await pending.result {
    case .success(let value): #expect(value != snapshot)
    case .failure(let error): #expect(error as? PluginError == .cancelled)
    }
}
