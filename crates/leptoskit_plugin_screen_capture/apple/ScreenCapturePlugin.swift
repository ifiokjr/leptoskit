import Foundation
import LeptosKitBridge
#if canImport(UIKit)
import UIKit
#endif

/// Apple observation uses public scene traits. It never claims screenshot prevention.
@MainActor
public final class ScreenCapturePlugin: NativePlugin {
    public let name = "screen-capture"
    public let methods: [UInt32] = [0, 1, 2, 3]
    private let context: NativePluginContext
    private let observation = CaptureObservation()
    private var closed = false

    /// The wire methods the Rust crate calls.
    private enum Method: UInt32 {
        case status = 0
        case protection = 1
        case screenshot = 2
        case prevent = 3
    }

    public init(context: NativePluginContext) { self.context = context }

    /// Answers one call: a status query works everywhere, and the rest need a
    /// presented scene UIKit can observe.
    public func invoke(method: UInt32, payload: NativeBuffer) async throws -> Data {
        guard !closed else { throw PluginError.cancelled }

        guard methods.contains(method) else { throw PluginError.unavailable }
        let watch = try readWatch(method, payload)

        guard !Task.isCancelled else { throw PluginError.cancelled }

        return try await answer(method, watch)
    }

    public func close() {
        closed = true
        observation.cancel()
    }

    /// Reads the watch a screenshot request carries; every other method carries none.
    private func readWatch(_ method: UInt32, _ payload: NativeBuffer) throws -> CaptureWatch? {
        let expected = method == Method.screenshot.rawValue ? CaptureWatch.bytes : 0

        guard payload.count == expected else { throw PluginError.invalidArgument }

        if method != Method.screenshot.rawValue { return nil }

        return try payload.withUnsafeBytes { try CaptureWatch($0) }
    }

    #if canImport(UIKit)
    /// Answers one call against the scene the presenting owner shows.
    private func answer(_ method: UInt32, _ watch: CaptureWatch?) async throws -> Data {
        if method == Method.status.rawValue { return captureFields([1, 0, 2, 3]) }

        if method == Method.prevent.rawValue { throw PluginError.unavailable }
        let (window, scene) = try presentedScene()

        if method == Method.protection.rawValue { return captureFields([1, state(scene), 0]) }

        guard let watch else { throw PluginError.invalidArgument }

        return try await observation.wait { deliver in
            self.observe(window: window, scene: scene, watch: watch, deliver: deliver)
        }
    }

    /// The visible window and the scene its capture state belongs to.
    private func presentedScene() throws -> (UIWindow, UIWindowScene) {
        let owner = try context.requirePresentationOwner()

        guard let window = owner.viewIfLoaded?.window,
              let scene = window.windowScene else { throw PluginError.unavailable }

        return (window, scene)
    }

    private func state(_ scene: UIWindowScene) -> UInt32 {
        switch scene.traitCollection.sceneCaptureState {
        case .active: 2
        case .inactive: 1
        case .unspecified: 0
        @unknown default: 0
        }
    }

    private func observe(
        window: UIWindow, scene: UIWindowScene, watch: CaptureWatch,
        deliver: @escaping CaptureObservation.Delivery
    ) -> () -> Void {
        var observers = departureObservers(scene: scene, deliver: deliver)
        let sample = sampleHandler(window: window, scene: scene, watch: watch, deliver: deliver)
        var traits: (any UITraitChangeRegistration)?

        if watch.kind == CaptureWatch.state {
            traits = stateObserver(scene: scene, sample: sample)
        } else {
            observers.append(screenshotObserver(sample: sample))
        }

        return { [weak scene] in

            for token in observers { NotificationCenter.default.removeObserver(token) }

            if let traits { scene?.unregisterForTraitChanges(traits) }
        }
    }

    /// The notifications that end a watch when the scene leaves the foreground.
    private func departureObservers(
        scene: UIWindowScene,
        deliver: @escaping CaptureObservation.Delivery
    ) -> [NSObjectProtocol] {
        var observers: [NSObjectProtocol] = []

        for name in [UIScene.willDeactivateNotification, UIScene.didDisconnectNotification] {
            observers.append(NotificationCenter.default.addObserver(forName: name, object: scene, queue: .main) { _ in
                MainActor.assumeIsolated { deliver(.failure(PluginError.cancelled)) }
            })
        }

        return observers
    }

    /// Watches the scene's capture state and samples it once after installing.
    private func stateObserver(
        scene: UIWindowScene,
        sample: @escaping @MainActor () -> Void
    ) -> any UITraitChangeRegistration {
        let traits = scene.registerForTraitChanges(
            [UITraitSceneCaptureState.self]
        ) { (_: UIWindowScene, _: UITraitCollection) in
            sample()
        }
        // Sample after installing the observer; latest state can coalesce transitions.
        sample()

        return traits
    }

    /// Watches the app-wide screenshot notification for one scene-owned watch.
    private func screenshotObserver(sample: @escaping @MainActor () -> Void) -> NSObjectProtocol {
        NotificationCenter.default.addObserver(
            forName: UIApplication.userDidTakeScreenshotNotification, object: nil, queue: .main
        ) { _ in MainActor.assumeIsolated { sample() } }
    }

    /// Reads the scene again when the OS reports it may have changed.
    private func sampleHandler(
        window: UIWindow,
        scene: UIWindowScene,
        watch: CaptureWatch,
        deliver: @escaping CaptureObservation.Delivery
    ) -> @MainActor () -> Void {
        return { [weak self, weak window, weak scene] in

            guard let self, let window, let scene else {
                deliver(.failure(PluginError.unavailable))

                return
            }

            self.sample(window: window, scene: scene, watch: watch, deliver: deliver)
        }
    }

    /// Answers one observation: a screenshot taken, or a changed capture state.
    private func sample(
        window: UIWindow,
        scene: UIWindowScene,
        watch: CaptureWatch,
        deliver: @escaping CaptureObservation.Delivery
    ) {
        do {
            try requirePresented(window, scene)
        } catch {
            deliver(.failure(error))

            return
        }

        if watch.kind == CaptureWatch.screenshot {
            // UIKit's screenshot notification is app-wide, not scene-attributed.
            deliver(.success(captureFields([1, 1, 0])))

            return
        }

        let current = state(scene)

        if current != watch.previous { deliver(.success(captureFields([1, 0, current]))) }
    }

    /// Rejects a sample for a window the presenting owner no longer shows.
    private func requirePresented(_ window: UIWindow, _ scene: UIWindowScene) throws {
        let owner = try context.requirePresentationOwner()

        guard owner.viewIfLoaded?.window === window, window.windowScene === scene else {
            throw PluginError.unavailable
        }
    }
    #else
    /// Only the status query is answered without UIKit.
    private func answer(_ method: UInt32, _ watch: CaptureWatch?) async throws -> Data {
        if method == Method.status.rawValue { return captureFields([1, 0, 0, 0]) }

        throw PluginError.unavailable
    }
    #endif
}
