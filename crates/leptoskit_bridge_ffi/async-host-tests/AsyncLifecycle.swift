import AsyncTest
import Foundation

enum PluginError: Error {
    case busy
    case invalidArgument
}

@main
struct AsyncLifecycle {
    @MainActor
    static func immediate() async {
        let operation = NativeRustAsync(taking: test_async_create(0))
        let reply = await operation.run()
        precondition(reply.complete(on: OpaquePointer(bitPattern: 1)!, id: 1) == 0)
    }

    @MainActor
    static func cancelledBatch() async throws {
        let budget = NativeJobBudget()
        var jobs: [Task<Void, Never>] = []
        for id in 1...128 {
            try budget.acquire(id: UInt64(id), payloadBytes: 0)
            let operation = NativeRustAsync(taking: test_async_create(1))
            jobs.append(Task {
                _ = await operation.run()
                budget.release(id: UInt64(id))
            })
        }
        let deadline = ContinuousClock.now.advanced(by: .seconds(5))
        while test_async_started() < 129 {
            precondition(ContinuousClock.now < deadline, "async jobs did not start")
            await Task.yield()
        }
        for job in jobs { job.cancel() }
        precondition(test_async_cancelled() == 128)
        precondition(!budget.canTakeRequest, "cancellation released capacity before cleanup")
        // A foreign callback owns no app and does not touch the UI actor.
        test_async_finish()
        for job in jobs { await job.value }
        precondition(budget.canTakeRequest)
    }

    @MainActor
    static func main() async throws {
        let watchdog = Task {
            do { try await Task.sleep(for: .seconds(10)) } catch { return }
            fatalError("async callback stranded a continuation")
        }
        defer { watchdog.cancel() }
        await immediate()
        try await cancelledBatch()
        precondition(test_async_live_jobs() == 0)
        precondition(test_async_live_replies() == 0)
        precondition(test_async_live_tokens() == 0)
        print("immediate and foreign async callbacks preserve cancellation ownership and capacity")
    }
}
