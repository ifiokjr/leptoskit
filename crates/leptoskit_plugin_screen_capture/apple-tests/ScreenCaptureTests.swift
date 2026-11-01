import Foundation
import LeptosKitBridge
@testable import LeptosKitScreenCapture
import Testing

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
    var first: CaptureObservation.Delivery?
    var removals = 0
    let cancelled = Task {
        try await observation.wait { delivery in
            first = delivery

            return { removals += 1 }
        }
    }
    defer { cancelled.cancel() }

    for _ in 0..<100 where first == nil { await Task.yield() }
    let stale = try #require(first)
    do {
        _ = try await observation.wait { _ in Issue.record("Overlapping observer installed"); return {} }
        Issue.record("Overlapping waiter accepted")
    } catch { #expect(error as? PluginError == .busy) }

    cancelled.cancel()

    do { _ = try await cancelled.value; Issue.record("Cancelled wait completed") } catch {
        #expect(error as? PluginError == .cancelled)
    }

    #expect(removals == 1)
    var second: CaptureObservation.Delivery?
    let replacement = Task {
        try await observation.wait { delivery in second = delivery; return { removals += 1 } }
    }
    defer { replacement.cancel() }

    for _ in 0..<100 where second == nil { await Task.yield() }
    let current = try #require(second)
    stale(.success(Data([9])))
    current(.success(Data([7])))

    #expect(try await replacement.value == Data([7]))
    #expect(removals == 2)
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

    for _ in 0..<100 where !installed { await Task.yield() }
    #expect(installed)
    observation.cancel()

    do { _ = try await pending.value; Issue.record("Close left an observation live") } catch {
        #expect(error as? PluginError == .cancelled)
    }

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
    do {
        _ = try await pending.value
        Issue.record("Cancelled request succeeded")
    } catch {
        #expect(error as? PluginError == .cancelled)
    }
}
