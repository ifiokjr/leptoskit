import Foundation
import LeptosKitBridge
#if os(iOS)
import AppTrackingTransparency
import UIKit
#endif

@MainActor
final class SystemTrackingAuthorization: TrackingAuthorizationProvider {
    private let context: NativePluginContext
    init(context: NativePluginContext) { self.context = context }

    func status() throws -> UInt32 {
        #if os(iOS)
        switch ATTrackingManager.trackingAuthorizationStatus {
        case .notDetermined: return 0
        case .restricted: return 1
        case .denied: return 2
        case .authorized: return 3
        @unknown default: throw PluginError.native
        }
        #else
        throw PluginError.unavailable
        #endif
    }

    func prepare(onBackground: @escaping @MainActor @Sendable () -> Void) throws -> TrackingObservation {
        #if os(iOS)
        let owner = try context.requirePresentationOwner()
        guard UIApplication.shared.applicationState == .active,
              let scene = owner.viewIfLoaded?.window?.windowScene else { throw PluginError.unavailable }
        try Self.validatePurpose(Bundle.main.object(forInfoDictionaryKey: "NSUserTrackingUsageDescription"))
        var observers: [NSObjectProtocol] = []
        // ATT's own modal UI may temporarily deactivate the scene. Only true
        // backgrounding or disconnect cancels its caller.
        for name in [UIScene.didEnterBackgroundNotification, UIScene.didDisconnectNotification] {
            observers.append(NotificationCenter.default.addObserver(forName: name, object: scene, queue: .main) { _ in
                MainActor.assumeIsolated { onBackground() }
            })
        }
        return TrackingObservation {
            for observer in observers { NotificationCenter.default.removeObserver(observer) }
        }
        #else
        throw PluginError.unavailable
        #endif
    }

    static func validatePurpose(_ value: Any?) throws {
        guard let purpose = value as? String,
              !purpose.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            // Calling ATT without this app-owned declaration may terminate the app.
            throw PluginError.invalidArgument
        }
    }

    func request(completion: @escaping @MainActor @Sendable () -> Void) throws {
        #if os(iOS)
        ATTrackingManager.requestTrackingAuthorization { _ in
            Task { @MainActor in completion() }
        }
        #else
        throw PluginError.unavailable
        #endif
    }
}
