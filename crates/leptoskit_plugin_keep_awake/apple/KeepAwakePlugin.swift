import Foundation
import LeptosKitBridge
#if os(iOS)
import UIKit
#endif

@MainActor
public final class KeepAwakePlugin: NativePlugin {
    public let name = "keep-awake"
    public let methods: [UInt32] = [0]
    private var pending: [UUID: CheckedContinuation<Data, any Error>] = [:]
    var activeLeaseCount: Int { pending.count }
    private var closed = false
    #if os(iOS)
    private var original = false
    private var backgroundObserver: NSObjectProtocol?
    #elseif os(macOS)
    private var assertion: NSObjectProtocol?
    #endif

    public init() {}

    public func invoke(method: UInt32, payload: NativeBuffer) async throws -> Data {
        guard !closed else { throw PluginError.cancelled }

        guard method == 0 else { throw PluginError.unavailable }

        guard payload.count == 0 else { throw PluginError.invalidArgument }

        guard pending.count < 64 else { throw PluginError.busy }
        try Task.checkCancellation()
        let identifier = UUID()

        return try await withTaskCancellationHandler {
            try await withCheckedThrowingContinuation { continuation in
                do {
                    if pending.isEmpty { try start() }
                    pending[identifier] = continuation
                } catch {
                    continuation.resume(throwing: error)
                }
            }
        } onCancel: { [weak self] in
            Task { @MainActor in self?.finish(identifier) }
        }
    }

    public func close() {
        closed = true
        finishAll()
    }

    private func finish(_ identifier: UUID) {
        guard let continuation = pending.removeValue(forKey: identifier) else { return }

        if pending.isEmpty { stop() }
        continuation.resume(throwing: PluginError.cancelled)
    }

    private func finishAll() {
        let continuations = Array(pending.values)
        pending.removeAll()
        stop()

        for continuation in continuations { continuation.resume(throwing: PluginError.cancelled) }
    }

    private func start() throws {
        #if os(iOS)

        guard UIApplication.shared.applicationState == .active else { throw PluginError.unavailable }
        original = UIApplication.shared.isIdleTimerDisabled
        UIApplication.shared.isIdleTimerDisabled = true
        backgroundObserver = NotificationCenter.default.addObserver(
            forName: UIApplication.didEnterBackgroundNotification, object: nil, queue: .main
        ) { [weak self] _ in
            MainActor.assumeIsolated { self?.finishAll() }
        }
        #elseif os(macOS)
        assertion = ProcessInfo.processInfo.beginActivity(
            options: [.idleDisplaySleepDisabled], reason: "LeptosKit display wake lock"
        )

        #else
        throw PluginError.unavailable
        #endif
    }

    private func stop() {
        #if os(iOS)

        guard let observer = backgroundObserver else { return }
        NotificationCenter.default.removeObserver(observer)
        backgroundObserver = nil
        UIApplication.shared.isIdleTimerDisabled = original
        #elseif os(macOS)

        if let assertion { ProcessInfo.processInfo.endActivity(assertion) }
        assertion = nil

        #endif
    }
}
