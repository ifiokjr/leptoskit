import CLeptosKitBridge
import Foundation

/// A copied runtime diagnostic remains available after native teardown.
public struct NativeAppError: Error, LocalizedError {
    public let message: String
    public init(message: String) { self.message = message }
    public var errorDescription: String? { message }
}

/// A joined native service failure. The native owner has completed cleanup.
public struct NativeAppCloseError: Error, LocalizedError {
    public let status: Int32
    public var errorDescription: String? { "Application service close failed (\(status))" }
}

/// Thread-safe notification storage contains only a generation token, never an App.
final class NativeServiceNotification: @unchecked Sendable {
    let token: UInt64
    private let lock = NSLock()
    private var queued = false

    init(token: UInt64) { self.token = token }

    @MainActor
    func retainedCallbacks() -> lk_service_callbacks {
        return lk_service_callbacks(
            context: Unmanaged.passRetained(self).toOpaque(),
            wake: { @Sendable context in
                guard let context else { return }
                Unmanaged<NativeServiceNotification>.fromOpaque(context).takeUnretainedValue().enqueue()
            },
            close: { context, handle in
                guard let context, let handle else { return }
                let token = Unmanaged<NativeServiceNotification>.fromOpaque(context).takeUnretainedValue().token
                MainActor.assumeIsolated { _ = NativeServiceRegistry.adopt(handle, token: token) }
            },
            release: { @Sendable context in
                if let context { Unmanaged<NativeServiceNotification>.fromOpaque(context).release() }
            }
        )
    }

    func enqueue() {
        lock.lock()

        guard !queued else { lock.unlock(); return }
        queued = true
        lock.unlock()
        DispatchQueue.main.async { [self] in
            lock.lock()
            queued = false
            lock.unlock()
            NativeServiceRegistry.poll(token)
        }
    }
}

/// Independently retained until native teardown joins, even if every waiter cancels.
@MainActor
public final class NativeAppClosing {
    private let pollNative: () -> Int32
    private let releaseNative: () -> Void
    private var result: Result<Void, NativeAppCloseError>?
    private var waiters: [CheckedContinuation<Void, Error>] = []

    init(poll: @escaping () -> Int32, release: @escaping () -> Void) {
        pollNative = poll
        releaseNative = release
    }

    /// Caller cancellation does not cancel the native join or release its interpreter.
    public func wait() async throws {
        if let result { return try result.get() }
        try await withCheckedThrowingContinuation { waiters.append($0) }
    }

    func poll() -> Bool {
        guard result == nil else { return true }
        let status = pollNative()

        guard status != 0 else { return false }
        finish(status)

        return true
    }

    private func finish(_ status: Int32) {
        let completed: Result<Void, NativeAppCloseError> = status > 0
            ? .success(()) : .failure(NativeAppCloseError(status: status))

        if status < 0 { NSLog("LeptosKit service close failed (%d)", status) }
        result = completed
        releaseNative()
        let pending = waiters

        waiters.removeAll()

        for waiter in pending { waiter.resume(with: completed.mapError { $0 as Error }) }
    }
}

/// App pointers and closing handles live exclusively in this main-actor registry.
@MainActor
enum NativeServiceRegistry {
    final class WeakOwner {
        weak var value: NativeAppLifecycle?
        init(_ value: NativeAppLifecycle) { self.value = value }
    }

    static var nextToken: UInt64 = 0
    static var active: [UInt64: WeakOwner] = [:]
    static var closing: [UInt64: NativeAppClosing] = [:]

    static func allocate() -> UInt64 {
        nextToken += 1

        return nextToken
    }

    static func poll(_ token: UInt64) {
        if let owner = closing[token] {
            if owner.poll() { closing.removeValue(forKey: token) }

            return
        }

        active[token]?.value?.onWake?()
    }

    static func adopt(_ handle: OpaquePointer, token: UInt64) -> NativeAppClosing {
        active.removeValue(forKey: token)
        let owner = NativeAppClosing(
            poll: { lk_app_close_poll(handle) },
            release: { lk_app_close_release(handle) }
        )
        closing[token] = owner
        // The callback may be inside App destruction; never poll it reentrantly.
        DispatchQueue.main.async { poll(token) }

        return owner
    }
}

/// UI-thread ownership installed before guest start. Wake callbacks only enqueue tokens.
@MainActor
public final class NativeAppLifecycle {
    /// Borrowed only until beginClose; never send this pointer to another thread.
    public let raw: OpaquePointer
    private var closeStarted = false
    public var onWake: (() -> Void)?
    private let token: UInt64
    private var closing: NativeAppClosing?

    public init(taking raw: OpaquePointer) throws {
        self.raw = raw
        token = NativeServiceRegistry.allocate()
        NativeServiceRegistry.active[token] = .init(self)
        let notification = NativeServiceNotification(token: token)
        let status = lk_app_activate_service(raw, notification.retainedCallbacks())

        guard status >= 0 else {
            let diagnostic = NativeBuffer(taking: lk_app_error(raw))
            _ = beginClose()
            throw NativeAppError(message: try diagnostic.utf8())
        }
    }

    isolated deinit {
        // Native Drop transfers pending cleanup into the installed retained scheduler.
        if !closeStarted { lk_app_destroy(raw) }
        NativeServiceRegistry.active.removeValue(forKey: token)
    }

    /// Stops admission synchronously and retains cleanup independently of the view.
    @discardableResult
    public func beginClose() -> NativeAppClosing {
        if let closing { return closing }
        onWake = nil
        NativeServiceRegistry.active.removeValue(forKey: token)
        let handle = lk_app_begin_close(raw)
        closeStarted = true
        let owner: NativeAppClosing

        if let handle {
            owner = NativeServiceRegistry.adopt(handle, token: token)

        } else {
            owner = NativeAppClosing(poll: { 1 }, release: {})
            _ = owner.poll()
        }

        closing = owner

        return owner
    }
}
