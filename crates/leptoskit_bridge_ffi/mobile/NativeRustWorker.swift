import CLeptosKitBridge
import Foundation

/// A retained Rust plugin and payload, with no app or interpreter reference.
/// The lock transfers its opaque ownership exactly once to a blocking worker.
final class NativeRustWorker: @unchecked Sendable {
    private static let executor: OperationQueue = {
        let queue = OperationQueue()
        queue.name = "dev.leptoskit.rust-workers"
        queue.qualityOfService = .userInitiated
        queue.maxConcurrentOperationCount = 4
        return queue
    }()
    private let lock = NSLock()
    private var handle: OpaquePointer?
    private let cancellation: OpaquePointer?
    private var cancelled = false

    init(taking handle: OpaquePointer) {
        self.handle = handle
        cancellation = lk_worker_cancellation(handle)
    }

    deinit {
        if let handle { lk_worker_release(handle) }
        lk_worker_cancellation_release(cancellation)
    }

    private func execute() -> NativeRustReply {
        lock.lock()
        let value = handle
        handle = nil
        let shouldRun = !cancelled
        lock.unlock()
        if shouldRun { return NativeRustReply(taking: lk_worker_execute(value)) }
        lk_worker_release(value)
        return NativeRustReply(taking: nil)
    }

    private func cancel() {
        lock.lock()
        cancelled = true
        lock.unlock()
        lk_worker_cancel(cancellation)
    }

    /// Blocking work never occupies Swift's cooperative executor. At most four
    /// operations execute concurrently; the host budgets queued jobs before
    /// calling this method. Cancellation still awaits actual completion.
    func run() async -> NativeRustReply {
        await withTaskCancellationHandler {
            await withCheckedContinuation { continuation in
                Self.executor.addOperation { [self] in
                    continuation.resume(returning: execute())
                }
            }
        } onCancel: {
            self.cancel()
        }
    }
}

/// An owned Rust result. A cancelled task releases it without accessing its app.
final class NativeRustReply: @unchecked Sendable {
    private let lock = NSLock()
    private var handle: OpaquePointer?

    init(taking handle: OpaquePointer?) { self.handle = handle }

    deinit {
        if let handle { lk_worker_reply_release(handle) }
    }

    private func take() -> OpaquePointer? {
        lock.lock()
        defer { lock.unlock() }
        let value = handle
        handle = nil
        return value
    }

    /// Transfer the Rust-owned bytes on the bridge's owning actor, without Data.
    @MainActor
    func complete(on bridge: OpaquePointer, id: UInt64) -> Int32 {
        lk_worker_complete(bridge, id, take())
    }
}
