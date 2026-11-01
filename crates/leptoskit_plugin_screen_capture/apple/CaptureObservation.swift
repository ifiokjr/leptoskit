import Foundation
import LeptosKitBridge

/// Owns one demand-driven subscription; every completion path removes it.
@MainActor
final class CaptureObservation {
    typealias Delivery = @MainActor (Result<Data, any Error>) -> Void
    private var pending: (UUID, CheckedContinuation<Data, any Error>)?
    private var cleanup: (() -> Void)?

    func wait(start: (@escaping Delivery) throws -> () -> Void) async throws -> Data {
        guard pending == nil else { throw PluginError.busy }

        guard !Task.isCancelled else { throw PluginError.cancelled }
        let identifier = UUID()

        return try await withTaskCancellationHandler {
            try await withCheckedThrowingContinuation { continuation in
                register(identifier, continuation, start)
            }
        } onCancel: {
            Task { @MainActor [weak self] in self?.finish(identifier, .failure(PluginError.cancelled)) }
        }
    }

    func cancel() {
        if let pending { finish(pending.0, .failure(PluginError.cancelled)) }
    }

    /// Starts the subscription, or answers at once when it cannot be installed.
    private func register(
        _ identifier: UUID,
        _ continuation: CheckedContinuation<Data, any Error>,
        _ start: (@escaping Delivery) throws -> () -> Void
    ) {
        guard !Task.isCancelled else {
            continuation.resume(throwing: PluginError.cancelled)

            return
        }

        pending = (identifier, continuation)

        do {
            let remove = try start { [weak self] result in self?.finish(identifier, result) }
            // A synchronous initial sample may complete during registration.
            if pending?.0 == identifier { cleanup = remove } else { remove() }
        } catch {
            finish(identifier, .failure(error))
        }
    }

    private func finish(_ identifier: UUID, _ result: Result<Data, any Error>) {
        guard let pending, pending.0 == identifier else { return }
        self.pending = nil
        let remove = cleanup
        cleanup = nil
        remove?()
        pending.1.resume(with: result)
    }
}

struct CaptureWatch {
    /// A request carries a kind and a previous outcome.
    static let bytes = 8
    /// The watch reports a changed capture state.
    static let state: UInt32 = 0
    /// The watch reports one screenshot taken.
    static let screenshot: UInt32 = 1
    /// The scene state the caller saw last.
    private static let previousLimit: UInt32 = 2

    let kind: UInt32
    let previous: UInt32

    init(_ bytes: UnsafeRawBufferPointer) throws {
        guard bytes.count == Self.bytes else { throw PluginError.invalidArgument }
        kind = UInt32(littleEndian: bytes.loadUnaligned(as: UInt32.self))
        previous = UInt32(littleEndian: bytes.loadUnaligned(fromByteOffset: 4, as: UInt32.self))

        guard kind <= Self.screenshot, previous <= Self.previousLimit else { throw PluginError.invalidArgument }

        guard kind == Self.state || previous == 0 else { throw PluginError.invalidArgument }
    }
}

func captureFields(_ values: [UInt32]) -> Data {
    var result = Data(capacity: values.count * 4)

    for value in values {
        var little = value.littleEndian
        withUnsafeBytes(of: &little) { result.append(contentsOf: $0) }
    }

    return result
}
