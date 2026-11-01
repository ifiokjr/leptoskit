@testable import LeptosKitBridge
import Foundation
import Testing

@Suite(.serialized)
@MainActor
struct IndependentLifecycleTests {
    private static let joinedFailure: Int32 = -17

    @Test func allWaitersObserveJoinedFailureDespiteCancellation() async {
        let token = NativeServiceRegistry.allocate()
        let signal = NativeServiceNotification(token: token)
        var ready = false
        var releases = 0
        let owner = NativeAppClosing(poll: { ready ? Self.joinedFailure : 0 }, release: { releases += 1 })
        NativeServiceRegistry.closing[token] = owner
        let first = Task { await status(owner) }
        let second = Task { await status(owner) }
        await nextTurn()
        first.cancel()
        NativeServiceRegistry.poll(token)
        #expect(releases == 0)
        ready = true

        signal.enqueue()
        #expect(await first.value == Self.joinedFailure)

        #expect(await second.value == Self.joinedFailure)
        #expect(releases == 1)
        #expect(await status(owner) == Self.joinedFailure)
    }

    @Test func wakeDuringPendingPollSchedulesAnotherTurn() async throws {
        let token = NativeServiceRegistry.allocate()
        let signal = NativeServiceNotification(token: token)
        var polls = 0
        var releases = 0
        let owner = NativeAppClosing(poll: {
            polls += 1
            if polls == 1 {
                signal.enqueue()
                return 0
            }
            return 1
        }, release: { releases += 1 })
        NativeServiceRegistry.closing[token] = owner
        signal.enqueue()
        try await owner.wait()

        #expect(polls == 2)
        #expect(releases == 1)
        signal.enqueue()
        await nextTurn()
        #expect(polls == 2)
    }

    private func status(_ owner: NativeAppClosing) async -> Int32 {
        do {
            try await owner.wait()

            return 0
        } catch let error as NativeAppCloseError {
            return error.status
        } catch {
            Issue.record("Unexpected error")

            return 1
        }
    }

    private func nextTurn() async {
        await withCheckedContinuation { continuation in
            DispatchQueue.main.async { continuation.resume() }
        }
    }
}
