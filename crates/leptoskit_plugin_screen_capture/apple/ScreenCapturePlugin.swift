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

    public init(context: NativePluginContext) { self.context = context }

    public func invoke(method: UInt32, payload: NativeBuffer) async throws -> Data {
        guard !closed else { throw PluginError.cancelled }

        guard methods.contains(method) else { throw PluginError.unavailable }

        guard payload.count == (method == 2 ? 8 : 0) else { throw PluginError.invalidArgument }
        let watch = method == 2 ? try payload.withUnsafeBytes { try CaptureWatch($0) } : nil

        guard !Task.isCancelled else { throw PluginError.cancelled }

        #if canImport(UIKit)

        if method == 0 { return captureFields([1, 0, 2, 3]) }

        if method == 3 { throw PluginError.unavailable }
        let owner = try context.requirePresentationOwner()

        guard let window = owner.viewIfLoaded?.window,
              let scene = window.windowScene else { throw PluginError.unavailable }

        if method == 1 { return captureFields([1, state(scene), 0]) }

        guard let watch else { throw PluginError.invalidArgument }

        return try await observation.wait { deliver in
            self.observe(window: window, scene: scene, watch: watch, deliver: deliver)
        }
        #else

        if method == 0 { return captureFields([1, 0, 0, 0]) }
        throw PluginError.unavailable
        #endif
    }

    public func close() {
        closed = true
        observation.cancel()
    }

    #if canImport(UIKit)
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
        let sample: @MainActor () -> Void = { [weak self, weak window, weak scene] in

            guard let self, let window, let scene else {
                deliver(.failure(PluginError.unavailable))

                return
            }

            do {
                let owner = try self.context.requirePresentationOwner()

                guard owner.viewIfLoaded?.window === window, window.windowScene === scene else {
                    throw PluginError.unavailable
                }

                if watch.kind == 1 {
                    // UIKit's screenshot notification is app-wide, not scene-attributed.
                    deliver(.success(captureFields([1, 1, 0])))
                } else {
                    let current = self.state(scene)

                    if current != watch.previous { deliver(.success(captureFields([1, 0, current]))) }
                }
            } catch {
                deliver(.failure(error))
            }
        }
        var observers: [NSObjectProtocol] = []

        for name in [UIScene.willDeactivateNotification, UIScene.didDisconnectNotification] {
            observers.append(NotificationCenter.default.addObserver(forName: name, object: scene, queue: .main) { _ in
                MainActor.assumeIsolated { deliver(.failure(PluginError.cancelled)) }
            })
        }

        var traits: (any UITraitChangeRegistration)?

        if watch.kind == 0 {

            traits = scene.registerForTraitChanges(
                [UITraitSceneCaptureState.self]
            ) { (_: UIWindowScene, _: UITraitCollection) in
                sample()
            }
            // Sample after installing the observer; latest state can coalesce transitions.
            sample()
        } else {
            observers.append(NotificationCenter.default.addObserver(
                forName: UIApplication.userDidTakeScreenshotNotification, object: nil, queue: .main
            ) { _ in MainActor.assumeIsolated { sample() } })
        }

        return { [weak scene] in

            for token in observers { NotificationCenter.default.removeObserver(token) }

            if let traits { scene?.unregisterForTraitChanges(traits) }
        }
    }
    #endif
}
