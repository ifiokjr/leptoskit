import Foundation
import LeptosKitBridge

/// A cancelled caller may retire while an external macOS service still owns UI.
@MainActor
final class ShareSession {
    let id = UUID()
    private var continuation: CheckedContinuation<Data, Error>?

    init(_ continuation: CheckedContinuation<Data, Error>) { self.continuation = continuation }

    func complete(_ result: Result<Data, Error>) {
        let pending = continuation
        continuation = nil
        pending?.resume(with: result)
    }
}
