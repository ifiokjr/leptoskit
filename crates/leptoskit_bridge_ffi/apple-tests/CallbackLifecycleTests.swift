@testable import LeptosKitBridge
import Foundation
import Testing
import WorkerTest

@Suite(.serialized)
@MainActor
struct CallbackLifecycleTests {
    private static let wakeCount: Int32 = 32

    @Test func workerCallbacksJoinCancelledWaiter() async throws {
        let watchdog = Task {
            do { try await Task.sleep(for: .seconds(10)) } catch { return }
            fatalError("native callback did not deliver joined cleanup")
        }
        defer { watchdog.cancel() }
        try await callbacksJoinAfterWaiterCancellation()
    }

    @MainActor
    private func callbacksJoinAfterWaiterCancellation() async throws {
        let token = NativeServiceRegistry.allocate()
        var notification: NativeServiceNotification? = NativeServiceNotification(token: token)
        weak let retained = notification
        let callbacks = notification!.retainedCallbacks()
        notification = nil
        precondition(retained != nil, "callback context was not retained")
        var polls = 0
        var releases = 0
        var ready = false
        let owner = NativeAppClosing(poll: {
            precondition(Thread.isMainThread, "native cleanup left the main thread")
            polls += 1
            return ready ? 1 : 0
        }, release: { releases += 1 })
        NativeServiceRegistry.closing[token] = owner
        let waiter = Task { try await owner.wait() }
        await nextMainTurn()
        waiter.cancel()
        test_service_wakes(callbacks, Self.wakeCount)
        await nextMainTurn()
        precondition(polls == 1, "worker wakes did not coalesce")
        precondition(releases == 0, "cancelled waiter released pending cleanup")
        ready = true
        test_service_wakes(callbacks, Self.wakeCount)
        try await waiter.value
        precondition(polls == 2 && releases == 1, "cleanup did not join exactly once")
        precondition(NativeServiceRegistry.closing[token] == nil)
        test_service_release(callbacks)
        precondition(retained == nil, "worker release leaked callback context")
        try await owner.wait()
        precondition(releases == 1, "repeated wait released native ownership twice")
    }

    @Test func retiredGenerationCannotPollReplacement() async {
        let retired = NativeServiceRegistry.allocate()
        let notification = NativeServiceNotification(token: retired)
        let callbacks = notification.retainedCallbacks()
        let replacement = NativeServiceRegistry.allocate()
        var polls = 0
        NativeServiceRegistry.closing[replacement] = NativeAppClosing(poll: {
            polls += 1
            return 1
        }, release: {})
        test_service_wakes(callbacks, Self.wakeCount)
        await nextMainTurn()
        precondition(polls == 0, "retired callback polled replacement generation")
        NativeServiceRegistry.poll(replacement)
        precondition(polls == 1)
        test_service_release(callbacks)
    }

    @MainActor
    private func nextMainTurn() async {
        await withCheckedContinuation { continuation in
            DispatchQueue.main.async { continuation.resume() }
        }
    }
}
