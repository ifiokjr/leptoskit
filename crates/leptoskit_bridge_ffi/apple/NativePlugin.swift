import CLeptosKitBridge
import Foundation

/// Stable native plugin failures, shared with Rust and Kotlin.
public enum PluginError: Int32, Error, Sendable {
    case unavailable = 1
    case permissionDenied = 2
    case invalidArgument = 3
    case busy = 4
    case tooLarge = 5
    case cancelled = 6
    case native = 7
    case invalidCall = 8
    case exhausted = 9
}

/// Immutable Rust-owned bytes. ARC releases the Rust allocation exactly once.
///
/// This is Sendable because the bytes are immutable and the owner is never
/// exposed. Retain this object while native asynchronous work reads its bytes.
public final class NativeBuffer: @unchecked Sendable {
    private let buffer: lk_buffer

    /// Transfers ownership from a successful native request/result call.
    /// The caller must never release or use `buffer` after this initializer.
    public init(taking buffer: lk_buffer) {
        self.buffer = buffer
    }

    deinit {
        lk_buffer_release(buffer)
    }

    public var count: Int { buffer.len }

    /// Borrows the original allocation without constructing a Data copy.
    public func withUnsafeBytes<Result>(
        _ body: (UnsafeRawBufferPointer) throws -> Result
    ) rethrows -> Result {
        try body(UnsafeRawBufferPointer(start: buffer.data, count: buffer.len))
    }

    public func utf8() throws -> String {
        try withUnsafeBytes { bytes in
            guard let value = String(bytes: bytes, encoding: .utf8) else {
                throw PluginError.invalidArgument
            }
            return value
        }
    }
}

/// The same plugin contract runs on the Apple UI actor and can suspend for
/// native permission dialogs or background work without blocking its run loop.
@MainActor
public protocol NativePlugin {
    var name: String { get }
    var methods: [UInt32] { get }
    func invoke(method: UInt32, payload: NativeBuffer) async throws -> Data
    /// Stop subscriptions and cancel native jobs before the guest is unloaded.
    func close()
}

public extension NativePlugin {
    func close() {}
}

/// Dispatches installed plugins. The embedding application owns the bridge,
/// grants capabilities, and completes replies on the bridge's owning thread.
@MainActor
public final class NativeDispatcher {
    private var plugins: [UInt32: any NativePlugin] = [:]

    public init() {}

    /// Installs a handler against a host-assigned identifier.
    public func install(id: UInt32, plugin: any NativePlugin) throws {
        guard plugins[id] == nil else { throw PluginError.invalidArgument }
        plugins[id] = plugin
    }

    /// Stops all plugin resources. The host must also cancel outstanding
    /// dispatcher Tasks and release the old bridge before installing a guest.
    public func close() {
        for plugin in plugins.values {
            plugin.close()
        }
        plugins.removeAll()
    }

    /// Takes ownership of the request payload even if dispatch fails.
    public func dispatch(_ request: lk_request) async throws -> Data {
        let payload = NativeBuffer(taking: request.payload)
        guard let plugin = plugins[request.plugin],
              plugin.methods.contains(request.method) else {
            throw PluginError.unavailable
        }
        return try await plugin.invoke(method: request.method, payload: payload)
    }
}
