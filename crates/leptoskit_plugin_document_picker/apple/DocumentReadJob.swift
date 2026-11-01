import Foundation

/// Blocking file coordination runs outside Swift's cooperative executor.
/// Cancellation is checked between reads; an in-flight OS read must finish
/// before this job releases its caller's presentation and host capacity.
final class DocumentReadJob: @unchecked Sendable {
    private static let executor: OperationQueue = {
        let queue = OperationQueue()
        queue.name = "dev.leptoskit.document-reads"
        queue.qualityOfService = .userInitiated
        queue.maxConcurrentOperationCount = 4
        return queue
    }()
    private let url: URL
    private let limit: Int
    private let lock = NSLock()
    private var cancelled = false

    init(url: URL, limit: Int) {
        self.url = url
        self.limit = limit
    }

    func cancel() {
        lock.lock()
        cancelled = true
        lock.unlock()
    }

    private func checkCancellation() throws {
        lock.lock()
        let cancelled = cancelled
        lock.unlock()

        if cancelled { throw CancellationError() }
    }

    func run(on executor: OperationQueue = DocumentReadJob.executor) async throws -> Data {
        try await withTaskCancellationHandler {
            try await withCheckedThrowingContinuation { continuation in
                executor.addOperation { [self] in
                    continuation.resume(with: Result {
                        try DocumentReader.read(url, limit: limit, checkCancellation: checkCancellation)
                    })
                }
            }
        } onCancel: {
            self.cancel()
        }
    }
}
