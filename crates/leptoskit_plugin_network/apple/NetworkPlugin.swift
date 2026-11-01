import Foundation
import LeptosKitBridge
import Network

/// The wire snapshot: eight little-endian u32 fields.
private enum Wire {
    static let snapshotBytes = 32
    static let fieldBytes = 4
    static let fieldCount = 8

    /// Field offsets inside the reading.
    static let connectionField = 0
    static let firstFlagField = 1
    static let lastFlagField = 4
    static let effectiveTypeField = 7

    /// The highest code each field may carry.
    static let lastConnection: UInt32 = 7
    static let lastFlag: UInt32 = 2
    static let lastEffectiveType: UInt32 = 4

    /// Tri-state encodings shared by every boolean field.
    static let flagUnknown: UInt32 = 0
    static let flagFalse: UInt32 = 1
    static let flagTrue: UInt32 = 2

    /// Numeric estimates use the reserved maximum for unknown.
    static let unknown: UInt32 = .max

    /// Method identifiers.
    static let snapshot: UInt32 = 0
    static let waitForChange: UInt32 = 1

    /// Connection codes the wire snapshot defines.
    static let none: UInt32 = 1
    static let wifi: UInt32 = 2
    static let cellular: UInt32 = 3
    static let ethernet: UInt32 = 4
    static let other: UInt32 = 7
}

/// Reads NWPath state and waits for the OS to report a change.
@MainActor
public final class NetworkPlugin: NativePlugin {
    public let name = "network"
    public let methods: [UInt32] = [Wire.snapshot, Wire.waitForChange]

    /// The single in-flight change request. A second one receives `busy`.
    private struct Pending {
        let identifier: UUID
        let monitor: NWPathMonitor
        let continuation: CheckedContinuation<Data, Error>
        let previous: Data?
    }

    private var pending: Pending?
    private var closed = false

    public init() {}

    public func invoke(method: UInt32, payload: NativeBuffer) async throws -> Data {
        guard !closed else { throw PluginError.cancelled }

        guard pending == nil else { throw PluginError.busy }

        return try await awaitChange(previous: previousReading(method: method, payload: payload))
    }

    /// Reads the reading each method compares against; a snapshot compares against nothing.
    private func previousReading(method: UInt32, payload: NativeBuffer) throws -> Data? {
        switch method {
        case Wire.snapshot:
            guard payload.count == 0 else { throw PluginError.invalidArgument }

            return nil
        case Wire.waitForChange:
            return try Self.previous(from: payload)
        default:
            throw PluginError.unavailable
        }
    }

    public func close() {
        closed = true

        if let pending { finish(identifier: pending.identifier, result: .failure(PluginError.cancelled)) }
    }

    /// Reads the caller's last reading, refusing fields the wire cannot carry.
    nonisolated private static func previous(from payload: NativeBuffer) throws -> Data {
        try payload.withUnsafeBytes { bytes in

            guard bytes.count == Wire.snapshotBytes else { throw PluginError.invalidArgument }

            let fields = (0..<Wire.fieldCount).map {
                UInt32(littleEndian: bytes.loadUnaligned(fromByteOffset: $0 * Wire.fieldBytes, as: UInt32.self))
            }

            guard fields[Wire.connectionField] <= Wire.lastConnection,
                  fields[Wire.firstFlagField...Wire.lastFlagField].allSatisfy({ $0 <= Wire.lastFlag }),
                  fields[Wire.effectiveTypeField] <= Wire.lastEffectiveType
            else {
                throw PluginError.invalidArgument
            }

            return Data(bytes)
        }
    }

    /// Starts a monitor and answers the first reading that differs from `previous`.
    private func awaitChange(previous: Data?) async throws -> Data {
        let identifier = UUID()

        return try await withTaskCancellationHandler {
            try Task.checkCancellation()

            return try await withCheckedThrowingContinuation { continuation in
                let monitor = NWPathMonitor()
                pending = Pending(
                    identifier: identifier, monitor: monitor, continuation: continuation, previous: previous
                )
                monitor.pathUpdateHandler = { [weak self] path in
                    let data = Self.encode(path)
                    Task { @MainActor [weak self] in
                        self?.updated(identifier: identifier, data: data)
                    }
                }
                monitor.start(queue: DispatchQueue(label: "dev.leptoskit.network"))
            }
        } onCancel: {
            Task { @MainActor [weak self] in
                self?.finish(identifier: identifier, result: .failure(PluginError.cancelled))
            }
        }
    }

    private func updated(identifier: UUID, data: Data) {
        guard let pending, pending.identifier == identifier, pending.previous != data else { return }
        finish(identifier: identifier, result: .success(data))
    }

    /// Answers the waiting caller once, then releases its monitor.
    private func finish(identifier: UUID, result: Result<Data, Error>) {
        guard let pending, pending.identifier == identifier else { return }
        self.pending = nil
        pending.monitor.pathUpdateHandler = nil
        pending.monitor.cancel()
        pending.continuation.resume(with: result)
    }

    /// Encodes a path; no probe or request is made, so reachability stays a hint.
    nonisolated private static func encode(_ path: NWPath) -> Data {
        let connected = path.status == .satisfied
        // A satisfied route does not prove internet access or server reachability.
        let fields: [UInt32] = [
            connection(path, connected: connected),
            connected ? Wire.flagTrue : Wire.flagFalse,
            Wire.flagUnknown,
            path.isExpensive ? Wire.flagTrue : Wire.flagFalse,
            path.isConstrained ? Wire.flagTrue : Wire.flagFalse,
            Wire.unknown,
            Wire.unknown,
            Wire.flagUnknown
        ]

        return fields.reduce(into: Data(capacity: Wire.snapshotBytes)) { result, field in
            var little = field.littleEndian
            withUnsafeBytes(of: &little) { result.append(contentsOf: $0) }
        }
    }

    /// Maps the path's interface to the wire connection code.
    nonisolated private static func connection(_ path: NWPath, connected: Bool) -> UInt32 {
        guard connected else { return Wire.none }

        if path.usesInterfaceType(.wifi) { return Wire.wifi }

        if path.usesInterfaceType(.cellular) { return Wire.cellular }

        if path.usesInterfaceType(.wiredEthernet) { return Wire.ethernet }

        return Wire.other
    }
}
