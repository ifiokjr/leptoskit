import Foundation
import LeptosKitBridge

/// The process has at most one underlying ATT request, including after its
/// caller has cancelled. Only the corresponding OS completion releases it.
@MainActor
final class TrackingRequestGate {
    static let shared = TrackingRequestGate()
    private var identifier: UUID?

    func acquire(_ identifier: UUID) throws {
        guard self.identifier == nil else { throw PluginError.busy }
        self.identifier = identifier
    }

    @discardableResult
    func complete(_ identifier: UUID) -> Bool {
        guard self.identifier == identifier else { return false }
        self.identifier = nil
        return true
    }
}

@MainActor
final class TrackingObservation {
    private var cancellation: (() -> Void)?
    init(_ cancellation: @escaping () -> Void = {}) { self.cancellation = cancellation }
    func stop() {
        let cancellation = self.cancellation
        self.cancellation = nil
        cancellation?()
    }
}

@MainActor
protocol TrackingAuthorizationProvider {
    func status() throws -> UInt32
    func prepare(onBackground: @escaping @MainActor @Sendable () -> Void) throws -> TrackingObservation
    func request(completion: @escaping @MainActor @Sendable () -> Void) throws
}
