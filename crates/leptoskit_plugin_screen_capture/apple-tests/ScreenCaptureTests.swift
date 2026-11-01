import Foundation
import LeptosKitBridge
@testable import LeptosKitScreenCapture
import Testing

/// Sample bytes the stale and current waits must not confuse.
private enum Sample {
    static let stale: UInt8 = 9
    static let current: UInt8 = 7
}

/// How many scheduler turns a registration gets before the test gives up.
private let attempts = 100

/// Shared state one observation test mutates from its callbacks.
@MainActor
private final class Recorder {
    var removals = 0
    var deliveries: [CaptureObservation.Delivery] = []
}

/// Starts a wait that records its delivery and counts the removal it hands back.
@MainActor
private func startWait(_ observation: CaptureObservation, _ recorder: Recorder) -> Task<Data, any Error> {
    return Task {
        try await observation.wait { delivery in
            recorder.deliveries.append(delivery)

            return { recorder.removals += 1 }
        }
    }
}

/// Yields until the wait at this index has registered its delivery.
@MainActor
private func awaitDelivery(
    _ recorder: Recorder,
    at index: Int
) async throws -> CaptureObservation.Delivery {
    for _ in 0..<attempts where recorder.deliveries.count <= index {
        await Task.yield()
    }

    return try #require(recorder.deliveries.count > index ? recorder.deliveries[index] : nil)
}

/// Expects a second wait to be refused while the first one owns the observation.
@MainActor
private func expectBusy(_ observation: CaptureObservation) async {
    do {
        _ = try await observation.wait { _ in
            Issue.record("Overlapping observer installed")

            return {}
        }
        Issue.record("Overlapping waiter accepted")
    } catch {
        #expect(error as? PluginError == .busy)
    }
}

/// Expects a wait to end with the cancelled error.
@MainActor
private func expectCancelled(_ task: Task<Data, any Error>) async {
    do {
        _ = try await task.value
        Issue.record("Cancelled wait completed")
    } catch {
        #expect(error as? PluginError == .cancelled)
    }
}

@Test
func watchDecoderRejectsMalformedEvents() throws {
    for values: [UInt32] in [[0, 3], [1, 1], [2, 0], [0], [1, 0, 0]] {
        #expect(throws: PluginError.invalidArgument) {
            try captureFields(values).withUnsafeBytes { try CaptureWatch($0) }
        }
    }

    #expect(try captureFields([0, 2]).withUnsafeBytes { try CaptureWatch($0).previous } == 2)
}

@Test
@MainActor
func cancellationRemovesObserversAndLateCallbacksCannotCompleteReplacement() async throws {
    let observation = CaptureObservation()
    let recorder = Recorder()
    let cancelled = startWait(observation, recorder)
    defer { cancelled.cancel() }
    let stale = try await awaitDelivery(recorder, at: 0)

    await expectBusy(observation)
    cancelled.cancel()
    await expectCancelled(cancelled)
    #expect(recorder.removals == 1)

    let replacement = startWait(observation, recorder)
    defer { replacement.cancel() }
    let current = try await awaitDelivery(recorder, at: 1)
    stale(.success(Data([Sample.stale])))
    current(.success(Data([Sample.current])))

    #expect(try await replacement.value == Data([Sample.current]))
    #expect(recorder.removals == 2)
}

@Test
@MainActor
func synchronousDeliveryFailureAndCloseReleaseSubscription() async throws {
    let observation = CaptureObservation()
    var removals = 0
    let result = try await observation.wait { deliver in
        deliver(.success(Data([1])))

        return { removals += 1 }
    }
    #expect(result == Data([1]))
    #expect(removals == 1)

    do {
        _ = try await observation.wait { _ in throw PluginError.unavailable }
        Issue.record("Failed observer registration succeeded")
    } catch { #expect(error as? PluginError == .unavailable) }
    var installed = false
    let pending = Task {
        try await observation.wait { _ in installed = true; return { removals += 1 } }
    }

    defer { pending.cancel() }

    for _ in 0..<attempts where !installed { await Task.yield() }
    #expect(installed)
    observation.cancel()
    await expectCancelled(pending)
    observation.cancel()
    #expect(removals == 2)
}

@Test
@MainActor
func preCancelledRequestNeverInstallsObserver() async throws {
    let observation = CaptureObservation()
    let pending = Task {
        try await observation.wait { _ in
            Issue.record("Cancelled request installed an observer")

            return {}
        }
    }
    pending.cancel()
    await expectCancelled(pending)
}
