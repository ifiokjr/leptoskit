import CLeptosKitBridge
import Foundation
import LeptosKitBridge
import Testing

@MainActor
private final class EchoPlugin: NativePlugin {
    let name = "echo"
    let methods: [UInt32] = [0]
    private(set) var closed = false

    func close() { closed = true }

    func invoke(method: UInt32, payload: NativeBuffer) async throws -> Data {
        #expect(method == 0)
        return Data(try payload.utf8().utf8)
    }
}

@Test
@MainActor
func swiftCanTransferRustBuffers() async throws {
    let bridge = try #require(lk_bridge_create())
    defer { lk_bridge_destroy(bridge) }
    let name = Array("echo".utf8)
    let methods: [UInt32] = [0]
    let plugin = name.withUnsafeBufferPointer { name in
        methods.withUnsafeBufferPointer { methods in
            lk_bridge_register(bridge, name.baseAddress, name.count, methods.baseAddress, methods.count)
        }
    }
    #expect(plugin == 0)
    #expect(lk_bridge_grant(bridge, 0, 0, true) == 0)
    let bytes = Array("hello".utf8)
    let call = bytes.withUnsafeBufferPointer {
        lk_bridge_submit(bridge, 0, 0, $0.baseAddress, $0.count)
    }
    #expect(call > 0)
    var request = lk_request()
    #expect(lk_bridge_next_request(bridge, &request) == 1)
    let dispatcher = NativeDispatcher()
    let echo = EchoPlugin()
    try dispatcher.install(id: 0, plugin: echo)
    let reply = try await dispatcher.dispatch(request)
    #expect(reply == Data("hello".utf8))
    let completed = reply.withUnsafeBytes {
        lk_bridge_complete(bridge, request.id, 0, $0.baseAddress?.assumingMemoryBound(to: UInt8.self), $0.count)
    }
    #expect(completed == 0)
    var response = lk_buffer()
    #expect(lk_bridge_take_result(bridge, request.id, &response) == 1)
    #expect(try NativeBuffer(taking: response).utf8() == "hello")
    #expect(lk_bridge_take_result(bridge, request.id, &response) == -8)
    dispatcher.close()
    #expect(echo.closed)
    dispatcher.close()
    let cancelled = lk_bridge_submit(bridge, 0, 0, nil, 0)
    #expect(cancelled > 0)
    #expect(lk_bridge_next_request(bridge, &request) == 1)
    lk_buffer_release(request.payload)
    #expect(lk_bridge_cancel(bridge, UInt64(cancelled)) == 0)
    #expect(lk_bridge_complete(bridge, UInt64(cancelled), 0, nil, 0) == -8)
    #expect(lk_bridge_next_cancelled(bridge) == cancelled)
    #expect(lk_bridge_next_cancelled(bridge) == 0)
}

@Test
@MainActor
func cancelledNativeWorkRetainsItsBudgetUntilItActuallyFinishes() async throws {
    let budget = NativeJobBudget()
    for identifier: UInt64 in 1...4 {
        try budget.acquire(id: identifier, payloadBytes: NativeJobBudget.maximumPayloadBytes)
    }
    #expect(!budget.canTakeRequest)
    var completion: CheckedContinuation<Void, Never>?
    // Model a native API that cannot abort its in-flight operation.
    let job = Task {
        await withCheckedContinuation { completion = $0 }
        budget.release(id: 1)
    }
    while completion == nil { await Task.yield() }
    job.cancel()
    await Task.yield()
    #expect(!budget.canTakeRequest)
    #expect(throws: PluginError.busy) { try budget.acquire(id: 5, payloadBytes: 0) }
    completion?.resume()
    await job.value
    #expect(budget.canTakeRequest)
    budget.release(id: 1) // Duplicate completion cannot over-credit the budget.
    try budget.acquire(id: 5, payloadBytes: NativeJobBudget.maximumPayloadBytes)
    #expect(!budget.canTakeRequest)
}

@Test
@MainActor
func emptyNativeCallsStillConsumeJobSlots() throws {
    let budget = NativeJobBudget()
    for identifier: UInt64 in 1...128 { try budget.acquire(id: identifier, payloadBytes: 0) }
    #expect(!budget.canTakeRequest)
    budget.release(id: 64)
    #expect(budget.canTakeRequest)
}
