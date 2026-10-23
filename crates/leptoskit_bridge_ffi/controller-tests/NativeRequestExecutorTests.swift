@testable import LeptosKitMacControllers
import CLeptosKitBridge
import Foundation
import LeptosKitBridge
import Testing

@MainActor
private final class GatedPlugin: NativePlugin {
    let name = "gated"
    let methods: [UInt32] = [0]
    var pending: [CheckedContinuation<Void, Never>] = []
    var closed = false

    func invoke(method: UInt32, payload: NativeBuffer) async throws -> Data {
        await withCheckedContinuation { pending.append($0) }

        return Data()
    }

    func close() { closed = true }

    func finish() {
        let waiting = pending
        pending.removeAll()

        for continuation in waiting { continuation.resume() }
    }
}

@Suite(.serialized)
@MainActor
struct NativeRequestExecutorTests {
    private static let maximumJobs = 128

    /// The moved executor retains budget and awaited jobs until native work really returns.
    @Test func cancelledJobsJoinBeforeCapacityReturns() async throws {
        let bridge = try #require(lk_bridge_create())
        defer { lk_bridge_destroy(bridge) }
        let executor = NativeRequestExecutor()
        let plugin = GatedPlugin()
        try executor.dispatcher.register(plugin, on: bridge)

        for _ in 0..<Self.maximumJobs {
            let call = lk_bridge_submit(bridge, 0, 0, nil, 0)
            #expect(call > 0)
            var request = lk_request()
            #expect(lk_bridge_next_request(bridge, &request) == 1)
            try executor.dispatch(request) { _ in nil }
        }

        while plugin.pending.count < Self.maximumJobs { await Task.yield() }
        #expect(!executor.canTakeRequest)
        executor.close()
        #expect(plugin.closed)
        var joined = false
        let waiting = Task {
            await executor.wait()
            joined = true
        }

        await Task.yield()
        #expect(!joined)

        #expect(!executor.canTakeRequest)
        plugin.finish()
        await waiting.value
        #expect(joined)
        #expect(executor.canTakeRequest)
    }

    /// Suspended request work does not retain the executor or its rendering owner.
    @Test func suspendedJobsDoNotRetainExecutor() async throws {
        let bridge = try #require(lk_bridge_create())
        defer { lk_bridge_destroy(bridge) }
        var executor: NativeRequestExecutor? = NativeRequestExecutor()
        weak var retained = executor
        let plugin = GatedPlugin()
        try executor?.dispatcher.register(plugin, on: bridge)
        #expect(lk_bridge_submit(bridge, 0, 0, nil, 0) > 0)
        var request = lk_request()

        #expect(lk_bridge_next_request(bridge, &request) == 1)
        try executor?.dispatch(request) { _ in nil }

        while plugin.pending.isEmpty { await Task.yield() }
        executor?.close()
        executor = nil
        #expect(retained == nil)
        plugin.finish()

        await Task.yield()
    }
}
