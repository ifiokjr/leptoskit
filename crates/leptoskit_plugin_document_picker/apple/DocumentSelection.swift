import Foundation

/// Continuations belong to one presentation; stale callbacks cannot consume a replacement.
@MainActor
final class DocumentSelection {
    private var continuation: CheckedContinuation<URL, Error>?
    private(set) var identifier: UUID?

    @discardableResult
    func begin(_ continuation: CheckedContinuation<URL, Error>) -> UUID {
        precondition(self.continuation == nil)
        let identifier = UUID()
        self.identifier = identifier
        self.continuation = continuation

        return identifier
    }

    func finish(_ result: Result<URL, Error>, for identifier: UUID? = nil) {
        if let identifier, identifier != self.identifier { return }
        let pending = continuation
        continuation = nil
        self.identifier = nil
        pending?.resume(with: result)
    }
}
