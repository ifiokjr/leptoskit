import CLeptosKitBridge
import Foundation
import LeptosKitBridge
@testable import LeptosKitTrackingTransparency
import XCTest

@MainActor
private final class FakeAuthorization: TrackingAuthorizationProvider {
    var value: UInt32 = 0
    var preparationError: PluginError?
    var requestError: PluginError?
    var requests = 0
    var stopped = 0
    var completion: (@MainActor @Sendable () -> Void)?
    var background: (@MainActor @Sendable () -> Void)?

    func status() throws -> UInt32 { value }
    func prepare(onBackground: @escaping @MainActor @Sendable () -> Void) throws -> TrackingObservation {
        if let preparationError { throw preparationError }
        background = onBackground

        return TrackingObservation { [weak self] in
            self?.stopped += 1
            self?.background = nil
        }
    }
    func request(completion: @escaping @MainActor @Sendable () -> Void) throws {
        if let requestError { throw requestError }
        requests += 1
        self.completion = completion
    }
}

@MainActor
final class TrackingTransparencyTests: XCTestCase {
    func testPurposeMustBeAnAppOwnedNonblankString() throws {
        let invalid: [Any?] = [nil, "", " \n\t ", 42, ["purpose"]]

        for value in invalid {
            XCTAssertThrowsError(try SystemTrackingAuthorization.validatePurpose(value)) { error in
                XCTAssertEqual(error as? PluginError, .invalidArgument)
            }
        }

        XCTAssertNoThrow(try SystemTrackingAuthorization.validatePurpose("Synthetic test purpose"))
    }

    private func call(_ plugin: TrackingTransparencyPlugin, method: UInt32 = 1) async throws -> UInt32 {
        let data = try await plugin.invoke(method: method, payload: NativeBuffer(taking: lk_buffer()))
        XCTAssertEqual(data.count, 4)

        return data.withUnsafeBytes { UInt32(littleEndian: $0.loadUnaligned(as: UInt32.self)) }
    }

    private func waitForRequest(_ provider: FakeAuthorization, count: Int = 1) async {
        for _ in 0..<1000 {
            if provider.requests >= count { return }
            await Task.yield()
        }

        XCTFail("Synthetic provider did not receive the expected request")
    }

    func testStatusPreservesEveryStateWithoutRequestingAndRejectsUnknown() async throws {
        let provider = FakeAuthorization()
        let plugin = TrackingTransparencyPlugin(provider: provider, gate: TrackingRequestGate())

        for status in UInt32(0)...3 {
            provider.value = status
            let actual = try await call(plugin, method: 0)
            XCTAssertEqual(actual, status)
        }

        XCTAssertEqual(provider.requests, 0)
        provider.value = 4
        do {
            _ = try await call(plugin, method: 0)
            XCTFail("Unknown status was accepted")
        } catch { XCTAssertEqual(error as? PluginError, .native) }
        plugin.close()
        do {
            _ = try await call(plugin, method: 0)
            XCTFail("Closed plugin returned a status")
        } catch { XCTAssertEqual(error as? PluginError, .cancelled) }
    }

    func testCancellationKeepsGlobalGateUntilLateCompletionAndAllowsReplacement() async throws {
        let gate = TrackingRequestGate()
        let provider = FakeAuthorization()
        let first = TrackingTransparencyPlugin(provider: provider, gate: gate)
        let other = TrackingTransparencyPlugin(provider: provider, gate: gate)
        let request = Task { try await self.call(first) }
        await waitForRequest(provider)
        request.cancel()
        do {
            _ = try await request.value
            XCTFail("Cancelled request returned a status")
        } catch { XCTAssertEqual(error as? PluginError, .cancelled) }

        XCTAssertEqual(provider.stopped, 1)
        do {
            _ = try await call(other)
            XCTFail("A cancelled OS request released its gate early")
        } catch { XCTAssertEqual(error as? PluginError, .busy) }
        let oldCompletion = provider.completion
        oldCompletion?()
        let replacement = Task { try await self.call(other) }
        await waitForRequest(provider, count: 2)
        oldCompletion?()
        do {
            _ = try await call(first)
            XCTFail("Duplicate old callback released the replacement gate")
        } catch { XCTAssertEqual(error as? PluginError, .busy) }
        provider.value = 3
        provider.completion?()
        let result = try await replacement.value
        XCTAssertEqual(result, 3)
        XCTAssertEqual(provider.stopped, 2)
    }

    func testCloseAndBackgroundEndCallerWithoutDeliveringLateAuthorization() async {
        for close in [false, true] {
            let provider = FakeAuthorization()
            let gate = TrackingRequestGate()
            let plugin = TrackingTransparencyPlugin(provider: provider, gate: gate)
            let request = Task { try await self.call(plugin) }
            await waitForRequest(provider)

            if close { plugin.close(); plugin.close() } else { provider.background?() }
            do {
                _ = try await request.value
                XCTFail("Stopped caller received a result")
            } catch { XCTAssertEqual(error as? PluginError, .cancelled) }

            XCTAssertEqual(provider.stopped, 1)
            provider.value = 3
            provider.completion?()
            XCTAssertEqual(provider.stopped, 1)
            let next = UUID()
            XCTAssertNoThrow(try gate.acquire(next))
            gate.complete(next)
        }
    }

    func testPreparationAndDispatchErrorsReleaseGateWithoutLaunchingPrompt() async {
        let provider = FakeAuthorization()
        let gate = TrackingRequestGate()
        let plugin = TrackingTransparencyPlugin(provider: provider, gate: gate)

        for failure in [PluginError.unavailable, .invalidArgument] {
            provider.preparationError = failure
            do {
                _ = try await call(plugin)
                XCTFail("Invalid request reached the system")
            } catch { XCTAssertEqual(error as? PluginError, failure) }
            XCTAssertEqual(provider.requests, 0)
        }

        provider.preparationError = nil

        provider.requestError = .native
        do {
            _ = try await call(plugin)
            XCTFail("Dispatch failure was hidden")
        } catch { XCTAssertEqual(error as? PluginError, .native) }
        XCTAssertEqual(provider.stopped, 1)
        let identifier = UUID()
        XCTAssertNoThrow(try gate.acquire(identifier))
        gate.complete(identifier)
    }

    func testExplicitRequestIsForwardedEvenAfterPriorDenial() async throws {
        let provider = FakeAuthorization()
        provider.value = 2
        let plugin = TrackingTransparencyPlugin(provider: provider, gate: TrackingRequestGate())
        let request = Task { try await self.call(plugin) }
        await waitForRequest(provider)
        provider.value = 0
        provider.completion?()
        let result = try await request.value

        XCTAssertEqual(result, 0, "Dismissal without a decision must not become authorization")
        XCTAssertEqual(provider.requests, 1)
    }
}
