import CLeptosKitBridge
import Foundation

/// Both Rust execution paths own their request independently of the UI runtime.
enum NativeRustOperation: Sendable {
    case worker(NativeRustWorker)
    case future(NativeRustAsync)

    @MainActor
    init?(app: OpaquePointer, request: inout lk_request) {
        if let worker = lk_app_worker(app, &request) {
            self = .worker(NativeRustWorker(taking: worker))
        } else if let future = lk_app_async(app, &request) {
            self = .future(NativeRustAsync(taking: future))
        } else {
            return nil
        }
    }

    /// The job the guest thread resolved for a request, if it has one.
    init?(worker: OpaquePointer?, job: OpaquePointer?) {
        if let worker {
            self = .worker(NativeRustWorker(taking: worker))
        } else if let job {
            self = .future(NativeRustAsync(taking: job))
        } else {
            return nil
        }
    }

    func run() async -> NativeRustReply {
        switch self {
        case .worker(let worker): return await worker.run()
        case .future(let future): return await future.run()
        }
    }
}

/// Retains a Rust future without retaining the app or occupying a worker thread.
final class NativeRustAsync: @unchecked Sendable {
    private let lock = NSLock()
    private var handle: OpaquePointer?
    private let cancellation: OpaquePointer?

    init(taking handle: OpaquePointer) {
        self.handle = handle
        cancellation = lk_async_cancellation_retain(handle)
    }

    deinit {
        lk_async_release(handle)
        lk_async_cancellation_release(cancellation)
    }

    private func take() -> OpaquePointer? {
        lock.lock()
        defer { lock.unlock() }
        let value = handle
        handle = nil

        return value
    }

    /// Cancellation requests cleanup; only the callback resumes the host job.
    /// Its admission budget therefore remains reserved until cleanup finishes.
    func run() async -> NativeRustReply {
        await withTaskCancellationHandler {
            await withCheckedContinuation { continuation in
                let context = Unmanaged.passRetained(AsyncCompletion(continuation)).toOpaque()
                lk_async_start(take(), context, completeAsync)
            }
        } onCancel: {
            lk_async_cancel(self.cancellation)
        }
    }
}

/// The foreign callback owns this context until Rust finishes, even after close.
private final class AsyncCompletion: Sendable {
    let continuation: CheckedContinuation<NativeRustReply, Never>
    init(_ continuation: CheckedContinuation<NativeRustReply, Never>) {
        self.continuation = continuation
    }
}

private func completeAsync(_ context: UnsafeMutableRawPointer?, _ reply: OpaquePointer?) {
    guard let context else {
        lk_worker_reply_release(reply)

        return
    }

    let completion = Unmanaged<AsyncCompletion>.fromOpaque(context).takeRetainedValue()
    completion.continuation.resume(returning: NativeRustReply(taking: reply))
}
