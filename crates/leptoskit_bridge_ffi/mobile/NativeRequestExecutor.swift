import CLeptosKitBridge
import Foundation
import LeptosKitBridge

/// Owns admitted plugin tasks and their budgets until actual completion.
@MainActor
final class NativeRequestExecutor {
    let dispatcher = NativeDispatcher()
    private let budget = NativeJobBudget()
    private var jobs: [UInt64: Task<Void, Never>] = [:]
    private var closingJobs: [Task<Void, Never>] = []
    var onCompletion: ((UInt64, NativeRequestResult) -> Void)?
    var onCapacity: (() -> Void)?
    var canTakeRequest: Bool { budget.canTakeRequest }

    func cancel(_ id: UInt64) { jobs[id]?.cancel() }

    func close() {
        closingJobs.append(contentsOf: jobs.values)

        for job in jobs.values { job.cancel() }
        dispatcher.close()
    }

    func wait() async {
        let pending = closingJobs

        for job in pending { await job.value }
        closingJobs.removeAll()
    }

    func dispatch(_ request: lk_request, worker: (inout lk_request) -> NativeRustOperation?) throws {
        let id = request.id
        do {
            try budget.acquire(id: id, payloadBytes: request.payload.len)
        } catch {
            lk_buffer_release(request.payload)
            throw error
        }
        var borrowed = request
        let worker = worker(&borrowed)

        if worker != nil { lk_buffer_release(request.payload) }
        let dispatcher = dispatcher
        let budget = budget
        jobs[id] = Task { [weak self] in
            defer {
                budget.release(id: id)
                self?.jobs.removeValue(forKey: id)
                self?.onCapacity?()
            }
            let result = await NativeRequestResult.run(request, worker: worker, dispatcher: dispatcher)

            if let result { self?.onCompletion?(id, result) }
        }
    }
}

/// Owned completion crosses suspension without retaining a rendering controller.
@MainActor
enum NativeRequestResult {
    case worker(NativeRustReply)
    case native(Result<Data, Error>)

    /// Suspend with owned request state so dropping a view can start its teardown.
    static func run(
        _ request: lk_request, worker: NativeRustOperation?, dispatcher: NativeDispatcher
    ) async -> NativeRequestResult? {
        guard !Task.isCancelled else {
            if worker == nil { lk_buffer_release(request.payload) }

            return nil
        }

        if let worker { return .worker(await worker.run()) }

        do { return .native(.success(try await dispatcher.dispatch(request))) } catch {
            return .native(.failure(error))
        }
    }
}
