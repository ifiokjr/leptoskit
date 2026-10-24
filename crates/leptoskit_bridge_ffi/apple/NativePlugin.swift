import CLeptosKitBridge
import Foundation

/// Stable native plugin failures, shared with Rust and Kotlin.
public enum PluginError: Int32, Error, Sendable {
    case unavailable = 1
    case permissionDenied
    case invalidArgument
    case busy
    case tooLarge
    case cancelled
    case native
    case invalidCall
    case exhausted
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

extension NativePlugin {
    public func close() {}
}

/// Dispatches installed plugins. The embedding application owns the bridge,
/// grants capabilities, and completes replies on the bridge's owning thread.
@MainActor
public final class NativeDispatcher {
    private var plugins: [UInt32: any NativePlugin] = [:]

    public init() {}

    /// Registers an explicitly selected plugin and grants its declared methods.
    public func register(_ plugin: any NativePlugin, on bridge: OpaquePointer) throws {
        let name = Array(plugin.name.utf8)
        let methods = plugin.methods
        let identifier = name.withUnsafeBufferPointer { name in
            methods.withUnsafeBufferPointer { methods in
                lk_bridge_register(
                    bridge, name.baseAddress, name.count, methods.baseAddress, methods.count)
            }
        }

        guard identifier >= 0, let id = UInt32(exactly: identifier) else {
            throw PluginError.invalidArgument
        }

        try install(id: id, plugin: plugin)

        for method in methods {
            guard lk_bridge_grant(bridge, id, method, true) == 0 else {
                throw PluginError.invalidArgument
            }
        }
    }

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
            plugin.methods.contains(request.method)
        else {
            throw PluginError.unavailable
        }

        return try await plugin.invoke(method: request.method, payload: payload)
    }
}
