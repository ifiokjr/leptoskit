/// Host accounting outlives guest cancellation: release a reservation only after
/// its native operation has actually stopped and released the request payload.
@MainActor
public final class NativeJobBudget {
    public static let maximumPayloadBytes = 8 * 1024 * 1024
    private var reservations: [UInt64: Int] = [:]
    private var bytes = 0

    public init() {}

    /// Reserve headroom for the largest bridge request before taking ownership.
    /// This conservatively pauses smaller requests too when fewer than 8 MiB remain.
    public var canTakeRequest: Bool {
        reservations.count < 128 && bytes <= 32 * 1024 * 1024 - Self.maximumPayloadBytes
    }

    public func acquire(id: UInt64, payloadBytes: Int) throws {
        guard canTakeRequest else { throw PluginError.busy }
        guard reservations[id] == nil, payloadBytes >= 0,
              payloadBytes <= Self.maximumPayloadBytes else { throw PluginError.invalidArgument }
        reservations[id] = payloadBytes
        bytes += payloadBytes
    }

    public func release(id: UInt64) {
        guard let size = reservations.removeValue(forKey: id) else { return }
        bytes -= size
    }
}
