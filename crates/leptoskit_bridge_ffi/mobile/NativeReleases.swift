import CLeptosKitBridge
import Foundation
import LeptosKitBridge

struct NativeLaunch: Decodable, Sendable {
    let receipt: String
    let version: String
    let build: UInt64
    let candidate: Bool
    let recovery: String?
}

struct NativeLaunchBundle: Sendable {
    let wasm: Data
    let metadata: NativeLaunch
}

struct NativeSyncReport: Decodable, Sendable {
    enum Status: String, Decodable, Sendable { case current, staged }
    let status: Status
    let version: String
    let build: UInt64
}

private struct ReleaseFailure: Error, LocalizedError {
    let message: String
    var errorDescription: String? { message }
}

/// Only owned data crosses this boundary. Every pointer access, including
/// destruction and blocking network I/O, executes on the same serial queue.
final class NativeReleases: Sendable {
    private let worker: ReleaseWorker

    init(configuration: Data, embedded: Data) {
        worker = ReleaseWorker(configuration: configuration, embedded: embedded)
    }

    func launch() async throws -> NativeLaunchBundle {
        try await worker.perform { state in
            let handle = try state.open()
            var wasm = lk_buffer()
            var metadata = lk_buffer()
            let status = lk_updates_launch(handle, &wasm, &metadata)
            defer {
                lk_buffer_release(wasm)
                lk_buffer_release(metadata)
            }
            try state.check(status)
            return NativeLaunchBundle(
                wasm: state.copy(wasm),
                metadata: try JSONDecoder().decode(NativeLaunch.self, from: state.copy(metadata))
            )
        }
    }

    func sync() async throws -> NativeSyncReport {
        try await worker.perform { state in
            var metadata = lk_buffer()
            let status = lk_updates_sync(try state.open(), &metadata)
            defer { lk_buffer_release(metadata) }
            try state.check(status)
            return try JSONDecoder().decode(NativeSyncReport.self, from: state.copy(metadata))
        }
    }

    func confirm(_ receipt: String) async throws {
        try await worker.perform { state in
            let handle = try state.open()
            try Data(receipt.utf8).withUnsafeBytes { bytes in
                try state.check(
                    lk_updates_confirm(
                        handle, bytes.bindMemory(to: UInt8.self).baseAddress,
                        bytes.count
                    ))
            }
        }
    }

    func reject(_ receipt: String) async throws {
        try await worker.perform { state in
            let handle = try state.open()
            try Data(receipt.utf8).withUnsafeBytes { bytes in
                try state.check(
                    lk_updates_reject(
                        handle, bytes.bindMemory(to: UInt8.self).baseAddress,
                        bytes.count
                    ))
            }
        }
    }

    func close() { worker.close() }

    /// The verified guest inside a store-shipped container, for running it without
    /// the update store. Any thread.
    static func entry(of embedded: Data) throws -> Data {
        var wasm = lk_buffer()
        var error = lk_buffer()
        let status = embedded.withUnsafeBytes { bytes in
            lk_bundle_entry(bytes.bindMemory(to: UInt8.self).baseAddress, bytes.count, &wasm, &error)
        }
        guard status == 0 else {
            throw ReleaseFailure(message: try NativeBuffer(taking: error).utf8())
        }
        return NativeBuffer(taking: wasm).withUnsafeBytes { Data($0) }
    }
}

private final class ReleaseWorker: Sendable {
    private let queue = DispatchQueue(label: "dev.leptoskit.releases", qos: .userInitiated)
    private let state: ReleaseState

    init(configuration: Data, embedded: Data) {
        state = ReleaseState(configuration: configuration, embedded: embedded)
    }

    deinit {
        let state = state
        queue.async { state.close() }
    }

    func perform<Value: Sendable>(
        _ action: @escaping @Sendable (ReleaseState) throws -> Value
    ) async throws -> Value {
        try await withCheckedThrowingContinuation { continuation in
            let state = state
            queue.async {
                do { continuation.resume(returning: try action(state)) } catch {
                    continuation.resume(throwing: error)
                }
            }
        }
    }

    func close() {
        let state = state
        queue.async { state.close() }
    }
}

// This type's mutable state is exclusively accessed by ReleaseWorker.queue.
// It never contains a guest/interpreter handle or calls back into UIKit.
private final class ReleaseState: @unchecked Sendable {
    private let configuration: Data
    private let embedded: Data
    private var handle: OpaquePointer?
    private var closed = false

    init(configuration: Data, embedded: Data) {
        self.configuration = configuration
        self.embedded = embedded
    }

    func open() throws -> OpaquePointer {
        guard !closed else { throw ReleaseFailure(message: "Release manager closed") }
        if let handle { return handle }
        let manager = FileManager.default
        var directory = try manager.url(
            for: .applicationSupportDirectory, in: .userDomainMask,
            appropriateFor: nil, create: true
        ).appendingPathComponent("LeptosKitReleases", isDirectory: true)
        try manager.createDirectory(at: directory, withIntermediateDirectories: true)
        var values = URLResourceValues()
        values.isExcludedFromBackup = true
        try directory.setResourceValues(values)
        let path = Data(directory.path.utf8)
        var error = lk_buffer()
        let raw = configuration.withUnsafeBytes { config in
            embedded.withUnsafeBytes { bundle in
                path.withUnsafeBytes { path in
                    lk_updates_open(
                        config.bindMemory(to: UInt8.self).baseAddress, config.count,
                        bundle.bindMemory(to: UInt8.self).baseAddress, bundle.count,
                        path.bindMemory(to: UInt8.self).baseAddress, path.count, &error
                    )
                }
            }
        }
        let failure = NativeBuffer(taking: error)
        guard let raw else {
            throw ReleaseFailure(message: try failure.utf8())
        }
        handle = raw
        return raw
    }

    func close() {
        if let handle { lk_updates_destroy(handle) }
        handle = nil
        closed = true
    }

    func check(_ status: Int32) throws {
        guard status >= 0 else {
            guard let handle else { throw ReleaseFailure(message: "Release manager unavailable") }
            throw ReleaseFailure(message: try NativeBuffer(taking: lk_updates_error(handle)).utf8())
        }
    }

    func copy(_ buffer: lk_buffer) -> Data {
        guard buffer.len > 0, let data = buffer.data else { return Data() }
        return Data(bytes: data, count: buffer.len)
    }
}
