import CLeptosKitBridge
import Foundation
import LeptosKitBridge

/// One main-actor guest owner: admission, service wake, event entry and joined close.
@MainActor
final class NativeAppRuntime {
    private static let batchLimit = 64
    private var app: NativeAppLifecycle?
    private var closeStarted = false
    private var closing: NativeAppClosing?
    private let requests = NativeRequestExecutor()
    private var pumpScheduled = false
    private var started = false
    private var lastSize = CGSize.zero
    let pluginContext: NativePluginContext
    var onRender: ((OpaquePointer) throws -> Void)?
    var onFailure: ((Error) -> Void)?

    var hasRive: Bool { app.map { lk_app_has_rive($0.raw) == 1 } ?? false }

    /// Display callbacks use only host-owned scenes, never the guest interpreter.
    func riveTick(_ elapsed: Double, scale: Float, flags: UInt32, callbacks: NativeRiveCallbacks) throws -> Bool {
        guard let app else { return false }
        let status = lk_app_rive_tick(
            app.raw, elapsed, scale, flags, callbacks.context, callbacks.visible, callbacks.present
        )
        try check(status)
        return status == 1
    }

    init(
        wasm: Data, identifier: String?, owner: NativePresentationOwner,
        register: (OpaquePointer, NativeDispatcher, NativePluginContext) throws -> Void
    ) throws {
        pluginContext = NativePluginContext(owner: owner)
        let app = try NativeAppFactory.create(wasm: wasm, identifier: identifier)
        self.app = app
        app.onWake = { [weak self] in self?.schedulePump() }
        requests.onCapacity = { [weak self] in self?.schedulePump() }
        requests.onCompletion = { [weak self] id, result in self?.finishRequest(id: id, result: result) }

        guard let bridge = lk_app_plugins(app.raw) else { throw PluginError.native }

        do { try register(bridge, requests.dispatcher, pluginContext) } catch {
            close()
            throw error
        }
    }

    isolated deinit {
        close()
    }

    /// The first positive layout starts the guest; later size changes resize it.
    func layout(size: CGSize, dark: Bool) throws {
        guard let app, size.width > 0, size.height > 0 else { return }

        if !started {
            started = true
            try check(lk_app_start(app.raw, Float(size.width), Float(size.height)))
        } else if lastSize != size {
            try check(lk_app_resize(app.raw, Float(size.width), Float(size.height), dark))
        }

        lastSize = size
        try pump()
    }

    func close() {
        guard !closeStarted else { return }
        closeStarted = true
        requests.close()
        pluginContext.invalidate()

        if let app { closing = app.beginClose() }
        app = nil
    }

    func closeAndWait() async throws {
        close()
        await requests.wait()
        try await closing?.wait()
    }

    private func check(_ status: Int32) throws {
        guard status >= 0 else {
            let message = app.map { NativeBuffer(taking: lk_app_error($0.raw)) }
            throw NativeAppError(message: try message?.utf8() ?? "Application closed")
        }
    }

    /// Poll service first, then cancellations, then admitted requests, then render.
    private func pump() throws {
        guard let app else { return }
        try check(lk_app_poll_service(app.raw))
        try drainCancellations(app)
        let more = try drainRequests(app)
        try onRender?(app.raw)

        if more { schedulePump() }
    }

    /// Cancellation is drained before any queued request can enter native work.
    private func drainCancellations(_ app: NativeAppLifecycle) throws {
        for index in 0..<Self.batchLimit {
            let cancelled = lk_app_next_cancelled(app.raw)

            if cancelled == 0 { return }

            guard cancelled > 0 else { throw PluginError.native }
            requests.cancel(UInt64(cancelled))

            if index == Self.batchLimit - 1 { schedulePump() }
        }
    }

    /// Capacity exhaustion yields to task completion; a full batch schedules another turn.
    private func drainRequests(_ app: NativeAppLifecycle) throws -> Bool {
        for _ in 0..<Self.batchLimit {
            guard requests.canTakeRequest else { return false }
            var request = lk_request()
            let status = lk_app_next_request(app.raw, &request)
            try check(status)

            if status == 0 { return false }

            if status == 2 { return true }
            try requests.dispatch(request) { borrowed in
                NativeRustOperation(app: app.raw, request: &borrowed)
            }
        }

        return true
    }

    private func schedulePump() {
        guard app != nil, started, !pumpScheduled else { return }
        pumpScheduled = true
        Task { [weak self] in
            self?.pumpScheduled = false

            do { try self?.pump() } catch { self?.onFailure?(error) }
        }
    }

    /// Ignore cancelled or closed delivery, preserving invalid-call cancellation semantics.
    private func finishRequest(id: UInt64, result: NativeRequestResult) {
        guard let app, !Task.isCancelled else { return }

        do {
            let status = try completeResult(result, id: id, app: app)

            if status == -PluginError.invalidCall.rawValue { return }
            try check(status)
            try check(lk_app_wake(app.raw))
        } catch { onFailure?(error) }
    }

    private func completeResult(_ result: NativeRequestResult, id: UInt64, app: NativeAppLifecycle) throws -> Int32 {
        switch result {
        case .worker(let reply):
            guard let bridge = lk_app_plugins(app.raw) else { throw PluginError.cancelled }

            return reply.complete(on: bridge, id: id)
        case .native(let reply): return try complete(id, reply)
        }
    }

    /// Foreign completion takes bytes synchronously before this guest can wake.
    private func complete(_ id: UInt64, _ result: Result<Data, Error>) throws -> Int32 {
        guard let app, let bridge = lk_app_plugins(app.raw) else { throw PluginError.cancelled }
        switch result {
        case .success(let data):
            return data.withUnsafeBytes {
                lk_bridge_complete(bridge, id, 0, $0.bindMemory(to: UInt8.self).baseAddress, $0.count)
            }
        case .failure(let error):
            let code = (error as? PluginError)?.rawValue ?? PluginError.native.rawValue

            if !(error is PluginError) { NSLog("LeptosKit plugin: %@", error.localizedDescription) }
            return lk_bridge_complete(bridge, id, code, nil, 0)
        }
    }

    func event(_ node: Int, _ kind: UInt32, _ text: String = "") {
        guard let app else { return }
        do {
            let bytes = Array(text.utf8)
            try check(bytes.withUnsafeBufferPointer {
                lk_app_event(app.raw, node, kind, $0.baseAddress, $0.count)
            })
            try pump()
        } catch { onFailure?(error) }
    }
}
