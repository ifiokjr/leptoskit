import Foundation
import LeptosKitBridge
import Network

@MainActor
public final class NetworkPlugin: NativePlugin {
    public let name = "network"
    public let methods: [UInt32] = [0, 1]
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

        guard method <= 1 else { throw PluginError.unavailable }
        let previous: Data?

        if method == 0 {
            guard payload.count == 0 else { throw PluginError.invalidArgument }
            previous = nil
        } else {
            previous = try payload.withUnsafeBytes { bytes in

                guard bytes.count == 32 else { throw PluginError.invalidArgument }
                let fields = (0..<8).map {
                    UInt32(littleEndian: bytes.loadUnaligned(fromByteOffset: $0 * 4, as: UInt32.self))
                }

                guard fields[0] <= 7, fields[1...4].allSatisfy({ $0 <= 2 }), fields[7] <= 4 else {

                    throw PluginError.invalidArgument
                }

                return Data(bytes)
            }
        }

        guard pending == nil else { throw PluginError.busy }
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

    public func close() {
        closed = true

        if let pending { finish(identifier: pending.identifier, result: .failure(PluginError.cancelled)) }
    }

    private func updated(identifier: UUID, data: Data) {
        guard let pending, pending.identifier == identifier, pending.previous != data else { return }
        finish(identifier: identifier, result: .success(data))
    }

    private func finish(identifier: UUID, result: Result<Data, Error>) {
        guard let pending, pending.identifier == identifier else { return }
        self.pending = nil
        pending.monitor.pathUpdateHandler = nil
        pending.monitor.cancel()
        pending.continuation.resume(with: result)
    }

    nonisolated private static func encode(_ path: NWPath) -> Data {
        let connected = path.status == .satisfied
        let connection: UInt32

        if !connected {
            connection = 1
        } else if path.usesInterfaceType(.wifi) {
            connection = 2
        } else if path.usesInterfaceType(.cellular) {
            connection = 3

        } else if path.usesInterfaceType(.wiredEthernet) {
            connection = 4
        } else {
            connection = 7
        }

        // A satisfied route does not prove internet access or server reachability.
        let fields: [UInt32] = [connection, connected ? 2 : 1, 0,
                                path.isExpensive ? 2 : 1, path.isConstrained ? 2 : 1,
                                UInt32.max, UInt32.max, 0]
        var result = Data(capacity: 32)

        for field in fields {
            var little = field.littleEndian
            withUnsafeBytes(of: &little) { result.append(contentsOf: $0) }
        }

        return result
    }
}
