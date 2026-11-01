import Foundation
import LeptosKitBridge

@MainActor
public final class TrackingTransparencyPlugin: NativePlugin {
    public let name = "tracking-transparency"
    public let methods: [UInt32] = [0, 1]
    private let provider: any TrackingAuthorizationProvider
    private let gate: TrackingRequestGate
    private var closed = false
    private var pending: Pending?

    private struct Pending {
        let identifier: UUID
        let continuation: CheckedContinuation<Data, any Error>
        let observation: TrackingObservation
    }

    public convenience init(context: NativePluginContext) {
        self.init(provider: SystemTrackingAuthorization(context: context), gate: .shared)
    }

    init(provider: any TrackingAuthorizationProvider, gate: TrackingRequestGate) {
        self.provider = provider
        self.gate = gate
    }

    public func invoke(method: UInt32, payload: NativeBuffer) async throws -> Data {
        guard !closed else { throw PluginError.cancelled }

        guard methods.contains(method) else { throw PluginError.unavailable }

        guard payload.count == 0 else { throw PluginError.invalidArgument }
        try Task.checkCancellation()

        if method == 0 { return try status() }
        let identifier = UUID()

        return try await withTaskCancellationHandler {
            try await withCheckedThrowingContinuation { continuation in
                begin(identifier, continuation: continuation)
            }
        } onCancel: {
            Task { @MainActor [weak self] in self?.cancel(identifier) }
        }
    }

    private func begin(_ identifier: UUID, continuation: CheckedContinuation<Data, any Error>) {
        guard !closed, !Task.isCancelled else {
            continuation.resume(throwing: PluginError.cancelled)

            return
        }

        do {
            try gate.acquire(identifier)
            let observation = try provider.prepare { [weak self] in self?.cancel(identifier) }
            pending = Pending(identifier: identifier, continuation: continuation, observation: observation)
            try provider.request { [weak self, gate] in

                guard gate.complete(identifier) else { return }
                self?.complete(identifier)
            }
        } catch {
            gate.complete(identifier)
            _ = take(identifier)
            continuation.resume(throwing: error)
        }
    }

    private func status() throws -> Data {
        var status = try provider.status()

        guard status <= 3 else { throw PluginError.native }
        status = status.littleEndian

        return withUnsafeBytes(of: &status) { Data($0) }
    }

    private func complete(_ identifier: UUID) {
        guard let pending = take(identifier) else { return }
        do {
            pending.continuation.resume(returning: try status())
        } catch {
            pending.continuation.resume(throwing: error)
        }
    }

    private func take(_ identifier: UUID) -> Pending? {
        guard let pending, pending.identifier == identifier else { return nil }
        self.pending = nil
        pending.observation.stop()

        return pending
    }

    private func cancel(_ identifier: UUID) {
        take(identifier)?.continuation.resume(throwing: PluginError.cancelled)
        // ATT has no dismissal operation. The gate remains held until the OS
        // callback, so cancellation cannot launch overlapping system prompts.
    }

    public func close() {
        guard !closed else { return }
        closed = true

        if let pending { cancel(pending.identifier) }
    }
}
