import CLeptosKitBridge
import Foundation
import LeptosKitBridge
import QuartzCore

/// Runs the guest on a thread of its own, so slow app logic never stops UIKit's scrolling, animation, or
/// touch handling (see `docs/agents/guest-thread.md` in the leptoskit repository).
///
/// This main-actor owner creates the app and registers its plugins, then lends it to the guest thread. It
/// posts the guest's inputs (layout, events, display frames, plugin replies) without waiting, and applies
/// what the guest posts back when the guest notifies it. Stopping the guest gives the app back to close.
@MainActor
final class NativeGuestRuntime {
    private var app: NativeAppLifecycle?
    private var guest: OpaquePointer?
    private var closeStarted = false
    private var closing: NativeAppClosing?
    private let requests = NativeRequestExecutor()
    private var started = false
    private var lastSize = CGSize.zero
    private lazy var frames = NativeFrameLink { [weak self] timestamp, period in
        self?.frame(timestamp: timestamp, period: period)
    }
    let pluginContext: NativePluginContext
    /// Applies one binary render transaction.
    var onTransaction: ((UnsafeRawBufferPointer) throws -> Void)?
    /// Presents one Rive frame: its view, size and premultiplied RGBA pixels.
    var onRiveFrame: ((Int, Int, Int, UnsafeBufferPointer<UInt8>) throws -> Void)?
    /// Called after each look at what the guest posted, with whether it owns Rive scenes.
    var onRive: ((Bool) -> Void)?
    var onFailure: ((Error) -> Void)?

    var hasRive: Bool { guest.map { lk_guest_has_rive($0) == 1 } ?? false }

    init(
        wasm: Data, identifier: String?, owner: NativePresentationOwner,
        register: (OpaquePointer, NativeDispatcher, NativePluginContext) throws -> Void
    ) throws {
        pluginContext = NativePluginContext(owner: owner)
        let app = try NativeAppFactory.create(wasm: wasm, identifier: identifier)
        self.app = app
        // This runtime drives the guest's frame clock from the display link.
        guard lk_app_enable_frames(app.raw) == 0 else { throw PluginError.native }
        app.onWake = { [weak self] in self?.wake() }
        requests.onCapacity = { [weak self] in self?.takeRequests() }
        requests.onCompletion = { [weak self] id, result in self?.finishRequest(id: id, result: result) }

        guard let bridge = lk_app_plugins(app.raw) else { throw PluginError.native }

        do { try register(bridge, requests.dispatcher, pluginContext) } catch {
            close()
            throw error
        }

        let notification = NativeGuestNotification { [weak self] in self?.drain() }
        let callbacks = notification.retainedCallbacks()

        guard let guest = lk_guest_spawn(app.raw, callbacks) else {
            Unmanaged<NativeGuestNotification>.fromOpaque(callbacks.context).release()
            close()
            throw NativeAppError(message: "Cannot start the guest thread")
        }

        self.guest = guest
    }

    isolated deinit {
        close()
    }

    /// The first positive layout starts the guest; later size changes resize it.
    func layout(size: CGSize, dark: Bool) throws {
        guard let guest, size.width > 0, size.height > 0 else { return }

        if !started {
            started = true
            try check(lk_guest_start(guest, Float(size.width), Float(size.height)))
        } else if lastSize != size {
            try check(lk_guest_resize(guest, Float(size.width), Float(size.height), dark))
        }

        lastSize = size
    }

    /// Stops the guest, waiting only for the step it is in, then closes the app it gives back.
    func close() {
        guard !closeStarted else { return }
        closeStarted = true
        frames.invalidate()
        requests.close()
        pluginContext.invalidate()

        if let guest, lk_guest_stop(guest) != 0 { NSLog("LeptosKit guest thread stopped abnormally") }

        guest = nil

        if let app { closing = app.beginClose() }
        app = nil
    }

    func closeAndWait() async throws {
        close()
        await requests.wait()
        try await closing?.wait()
    }

    func event(_ node: Int, _ kind: UInt32, _ text: String = "") {
        guard let guest else { return }
        let bytes = Array(text.utf8)
        let status = bytes.withUnsafeBufferPointer { lk_guest_event(guest, node, kind, $0.baseAddress, $0.count) }

        if status < 0 { fail() }
    }

    /// Asks the guest to advance its Rive scenes; the frames come back through ``onRiveFrame``. Returns
    /// whether the last tick the guest ran wanted another.
    func riveTick(_ elapsed: Double, scale: Float, flags: UInt32, visible: [Int]) throws -> Bool {
        guard let guest else { return false }
        let status = visible.withUnsafeBufferPointer {
            lk_guest_rive_tick(guest, elapsed, scale, flags, $0.baseAddress, $0.count)
        }

        guard status >= 0 else { throw NativeAppError(message: "Invalid native Rive display environment") }

        return status == 1
    }

    private func check(_ status: Int32) throws {
        guard status >= 0 else { throw NativeAppError(message: "Application closed") }
    }

    private func wake() {
        if let guest { _ = lk_guest_wake(guest) }
    }

    private func frame(timestamp: UInt64, period: UInt64) {
        if let guest { _ = lk_guest_frame(guest, timestamp, period) }
    }

    /// Takes everything the guest posted: cancellations before new requests, then requests the job budget
    /// admits, then transactions in order, then Rive frames.
    private func drain() {
        guard let guest else { return }
        lk_guest_ack(guest)

        guard lk_guest_failed(guest) == 0 else { return fail() }

        do {
            while case let cancelled = lk_guest_next_cancelled(guest), cancelled != 0 {
                guard cancelled > 0 else { throw PluginError.native }
                requests.cancel(UInt64(cancelled))
            }

            try takeRequestsOrThrow()

            while true {
                let transaction = NativeBuffer(taking: lk_guest_take_render(guest))

                guard transaction.count > 0 else { break }
                try transaction.withUnsafeBytes { try onTransaction?($0) }
            }

            try presentRiveFrames(guest)
            frames.run(lk_guest_frame_requested(guest) == 1)
            onRive?(lk_guest_has_rive(guest) == 1)
        } catch { onFailure?(error) }
    }

    private func takeRequests() {
        do { try takeRequestsOrThrow() } catch { onFailure?(error) }
    }

    private func takeRequestsOrThrow() throws {
        guard let guest else { return }

        while requests.canTakeRequest {
            var request = lk_request()
            var worker: OpaquePointer?
            var job: OpaquePointer?

            guard lk_guest_next_request(guest, &request, &worker, &job) == 1 else { return }
            // Owned before dispatch, so a refused request still releases its job.
            let operation = NativeRustOperation(worker: worker, job: job)
            try requests.dispatch(request) { _ in operation }
        }
    }

    private func presentRiveFrames(_ guest: OpaquePointer) throws {
        var node = 0
        var width: UInt32 = 0
        var height: UInt32 = 0

        while true {
            let pixels = NativeBuffer(taking: lk_guest_take_rive_frame(guest, &node, &width, &height))

            guard pixels.count > 0 else { return }
            try pixels.withUnsafeBytes {
                try onRiveFrame?(node, Int(width), Int(height), $0.bindMemory(to: UInt8.self))
            }
        }
    }

    /// Ignores cancelled or closed delivery, as the guest ignores a call it stopped waiting for.
    private func finishRequest(id: UInt64, result: NativeRequestResult) {
        guard let guest, !Task.isCancelled else { return }

        switch result {
        case .worker(let reply):
            _ = reply.complete(onGuest: guest, id: id)
        case .native(.success(let data)):
            _ = data.withUnsafeBytes {
                lk_guest_complete(guest, id, 0, $0.bindMemory(to: UInt8.self).baseAddress, $0.count)
            }
        case .native(.failure(let error)):
            let code = (error as? PluginError)?.rawValue ?? PluginError.native.rawValue

            if !(error is PluginError) { NSLog("LeptosKit plugin: %@", error.localizedDescription) }
            _ = lk_guest_complete(guest, id, code, nil, 0)
        }
    }

    private func fail() {
        guard let guest else { return }
        let message = (try? NativeBuffer(taking: lk_guest_error(guest)).utf8()) ?? "The guest failed"
        onFailure?(NativeAppError(message: message))
    }
}

/// Carries the guest thread's notifications to the main actor. The guest retains it until it is stopped.
final class NativeGuestNotification: @unchecked Sendable {
    private let onNotify: @MainActor () -> Void

    init(_ onNotify: @escaping @MainActor () -> Void) { self.onNotify = onNotify }

    func retainedCallbacks() -> lk_guest_callbacks {
        lk_guest_callbacks(
            context: Unmanaged.passRetained(self).toOpaque(),
            notify: { context in
                guard let context else { return }
                let owner = Unmanaged<NativeGuestNotification>.fromOpaque(context).takeUnretainedValue()
                DispatchQueue.main.async { MainActor.assumeIsolated { owner.onNotify() } }
            },
            release: { context in
                if let context { Unmanaged<NativeGuestNotification>.fromOpaque(context).release() }
            })
    }
}
