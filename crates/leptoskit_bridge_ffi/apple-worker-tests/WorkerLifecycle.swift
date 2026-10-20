import Foundation
import WorkerTest

// NativeJobBudget depends only on these two stable errors.
enum PluginError: Error {
    case busy
    case invalidArgument
}

@main
struct WorkerLifecycle {
    @MainActor
    static func main() async throws {
        let watchdog = Task {
            do { try await Task.sleep(for: .seconds(10)) } catch { return }
            fatalError("worker cancellation did not resume its continuation")
        }
        defer { watchdog.cancel() }
        let budget = NativeJobBudget()
        for id in 1...128 { try budget.acquire(id: UInt64(id), payloadBytes: 0) }
        precondition(!budget.canTakeRequest)
        let active = (0..<4).map { _ in
            let worker = NativeRustWorker(taking: test_worker_create())
            return Task { await worker.run() }
        }
        let deadline = ContinuousClock.now.advanced(by: .seconds(5))
        while test_worker_started() < 4 {
            precondition(ContinuousClock.now < deadline, "worker pool did not start")
            try await Task.sleep(for: .milliseconds(1))
        }
        let queued = NativeRustWorker(taking: test_worker_create())
        let cancelled = Task {
            let reply = await queued.run()
            budget.release(id: 5)
            return reply
        }
        await Task.yield()
        cancelled.cancel()
        precondition(!budget.canTakeRequest, "queued cancellation released capacity early")
        precondition(test_worker_started() == 4, "worker concurrency exceeded four")
        test_worker_unblock()
        _ = await cancelled.value
        for task in active { _ = await task.value }
        precondition(test_worker_started() == 4, "cancelled queued work invoked the plugin")
        precondition(test_worker_released() == 5, "worker ownership was leaked or released twice")
        precondition(budget.canTakeRequest, "completed queued cancellation did not release capacity")
        print("queued cancellation preserves worker ownership and host capacity")
    }
}
