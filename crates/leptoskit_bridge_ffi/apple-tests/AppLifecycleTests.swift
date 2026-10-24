@testable import LeptosKitBridge
import CLeptosKitBridge
import Foundation
import Testing

@Suite(.serialized)
@MainActor
struct AppLifecycleTests {
    private static let notificationCount = 32
    private static let joinedFailure: Int32 = -9

    /// Native Pending is observable before notification, with no idle polling.
    @Test func pendingCloseOnlyPollsOnWakeAndReleasesOnce() async throws {
        let token = NativeServiceRegistry.allocate()
        let signal = NativeServiceNotification(token: token)
        var ready = false
        var polls = 0
        var releases = 0
        let owner = NativeAppClosing(poll: {
            polls += 1
            return ready ? 1 : 0
        }, release: { releases += 1 })
        NativeServiceRegistry.closing[token] = owner
        NativeServiceRegistry.poll(token)
        #expect(polls == 1)
        #expect(releases == 0)
        await nextMainTurn()
        #expect(polls == 1)

        #expect(NativeServiceRegistry.closing[token] === owner)
        ready = true

        DispatchQueue.concurrentPerform(iterations: Self.notificationCount) { _ in signal.enqueue() }
        await nextMainTurn()
        try await owner.wait()
        #expect(polls == 2)
        #expect(releases == 1)
        #expect(NativeServiceRegistry.closing[token] == nil)
        #expect(owner.poll())
        try await owner.wait()
        #expect(polls == 2)

        #expect(releases == 1)
    }

    /// Cancelling a waiter or losing its view cannot discard retained cleanup.
    @Test func cancelledWaiterAndReleasedViewStillJoin() async throws {
        let token = NativeServiceRegistry.allocate()
        let signal = NativeServiceNotification(token: token)
        var ready = false
        var releases = 0
        var viewOwner: NativeAppClosing? = NativeAppClosing(
            poll: { ready ? 1 : 0 }, release: { releases += 1 }
        )

        weak let retained = viewOwner
        NativeServiceRegistry.closing[token] = viewOwner
        let waitingOwner = try #require(viewOwner)
        let waiter = Task { try await waitingOwner.wait() }

        await nextMainTurn()
        waiter.cancel()
        viewOwner = nil
        #expect(retained != nil)
        #expect(releases == 0)
        ready = true

        signal.enqueue()
        try await waiter.value
        #expect(releases == 1)
        #expect(NativeServiceRegistry.closing[token] == nil)
    }

    /// Joined errors stay available to every waiter; a late old wake is inert.
    @Test func joinedErrorIsStableAndLateWakeCannotPollReplacement() async throws {
        let oldToken = NativeServiceRegistry.allocate()
        let signal = NativeServiceNotification(token: oldToken)
        var releases = 0
        let owner = NativeAppClosing(poll: { Self.joinedFailure }, release: { releases += 1 })
        NativeServiceRegistry.closing[oldToken] = owner
        NativeServiceRegistry.poll(oldToken)

        await expectFailure(owner, status: Self.joinedFailure)
        await expectFailure(owner, status: Self.joinedFailure)

        let newToken = NativeServiceRegistry.allocate()
        var replacementPolls = 0
        let replacement = NativeAppClosing(poll: {
            replacementPolls += 1
            return 1
        }, release: {})
        NativeServiceRegistry.closing[newToken] = replacement
        signal.enqueue()
        await nextMainTurn()
        #expect(replacementPolls == 0)
        #expect(releases == 1)

        NativeServiceRegistry.poll(newToken)
        #expect(replacementPolls == 1)
    }

    /// The registry alone retains cleanup after a view disappears with no waiter.
    @Test func registryRetainsClosingOwnerWithoutAViewOrWaiter() async {
        let token = NativeServiceRegistry.allocate()
        let signal = NativeServiceNotification(token: token)
        var ready = false
        var releases = 0
        var viewOwner: NativeAppClosing? = NativeAppClosing(
            poll: { ready ? 1 : 0 }, release: { releases += 1 }
        )
        weak let retained = viewOwner
        NativeServiceRegistry.closing[token] = viewOwner
        viewOwner = nil

        NativeServiceRegistry.poll(token)
        #expect(retained != nil)
        #expect(releases == 0)
        ready = true
        signal.enqueue()
        await nextMainTurn()
        #expect(releases == 1)
        #expect(retained == nil)
    }

    /// The real native ABI consumes Swift's valid callback context on activation failure.
    @Test func nativeActivationFailureReleasesSwiftCallbackStorage() {
        var notification: NativeServiceNotification? = NativeServiceNotification(
            token: NativeServiceRegistry.allocate()
        )
        weak let retained = notification
        let callbacks = notification?.retainedCallbacks()
        notification = nil
        #expect(retained != nil)

        guard let callbacks else { Issue.record("Missing callbacks"); return }
        #expect(lk_app_activate_service(nil, callbacks) < 0)
        #expect(retained == nil)
    }

    private func expectFailure(_ owner: NativeAppClosing, status: Int32) async {
        do {
            try await owner.wait()
            Issue.record("Joined close failure was lost")
        } catch let failure as NativeAppCloseError {
            #expect(failure.status == status)
        } catch {
            Issue.record("Unexpected close error: \(error)")
        }
    }

    private func nextMainTurn() async {
        await withCheckedContinuation { continuation in
            DispatchQueue.main.async { continuation.resume() }
        }
    }
}
