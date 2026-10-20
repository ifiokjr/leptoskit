import Foundation
import LeptosKitBridge
#if canImport(UIKit)
import UIKit
#endif

/// A pending invocation owns one orientation request on its explicit host scene.
/// Cancellation restores the prior controller policy; OS geometry is advisory.
@MainActor
public final class ScreenOrientationPlugin: NativePlugin {
    public let name = "screen-orientation"
    public let methods: [UInt32] = [0, 1]
    private let context: NativePluginContext
    private var closed = false
    #if canImport(UIKit)
    private static var owners: [ObjectIdentifier: UUID] = [:]
    private var pending: (UUID, CheckedContinuation<Data, any Error>)?
    private var ownerKey: ObjectIdentifier?
    private var original: UIInterfaceOrientationMask?
    private var originalEffective: UIInterfaceOrientationMask = []
    private weak var scene: UIWindowScene?
    private weak var controller: UIViewController?
    private var observers: [NSObjectProtocol] = []
    #endif

    public init(context: NativePluginContext) { self.context = context }

    public func invoke(method: UInt32, payload: NativeBuffer) async throws -> Data {
        guard !closed else { throw PluginError.cancelled }
        guard methods.contains(method) else { throw PluginError.unavailable }
        guard payload.count == (method == 1 ? 4 : 0) else { throw PluginError.invalidArgument }
        try Task.checkCancellation()
        #if canImport(UIKit)
        let owner = try context.requirePresentationOwner()
        guard let window = owner.viewIfLoaded?.window,
              let target = window.windowScene else { throw PluginError.unavailable }
        if method == 0 { return snapshot(of: target) }
        let policy = try payload.withUnsafeBytes { try OrientationPolicy.decode($0) }
        let requested = policy.mask.intersection(UIApplication.shared.supportedInterfaceOrientations(for: window))
        guard !requested.isEmpty else { throw PluginError.unavailable }
        return try await hold(requested, on: target, owner: owner)
        #else
        throw PluginError.unavailable
        #endif
    }

    public func close() {
        guard !closed else { return }
        closed = true
        #if canImport(UIKit)
        if let pending { finish(pending.0, error: .cancelled) }
        #endif
    }

    #if canImport(UIKit)
    private func snapshot(of scene: UIWindowScene) -> Data {
        let orientation: UInt32
        switch scene.interfaceOrientation {
        case .portrait, .portraitUpsideDown: orientation = 1
        case .landscapeLeft, .landscapeRight: orientation = 2
        default: orientation = 0
        }
        return [orientation, UInt32.max].withUnsafeBytes { Data($0) }
    }

    private func hold(
        _ requested: UIInterfaceOrientationMask,
        on target: UIWindowScene,
        owner: UIViewController
    ) async throws -> Data {
        let key = ObjectIdentifier(target)
        guard pending == nil, Self.owners[key] == nil else { throw PluginError.busy }
        let identifier = UUID()
        return try await withTaskCancellationHandler {
            try await withCheckedThrowingContinuation { continuation in
                guard !Task.isCancelled else {
                    continuation.resume(throwing: PluginError.cancelled)
                    return
                }
                original = context.orientationMask
                originalEffective = owner.supportedInterfaceOrientations
                controller = owner
                scene = target
                pending = (identifier, continuation)
                ownerKey = key
                Self.owners[key] = identifier
                for name in [UIScene.willDeactivateNotification, UIScene.didDisconnectNotification] {
                    observers.append(NotificationCenter.default.addObserver(
                        forName: name, object: target, queue: .main
                    ) { [weak self] _ in
                        MainActor.assumeIsolated { self?.finish(identifier, error: .cancelled) }
                    })
                }
                context.orientationMask = requested
                owner.setNeedsUpdateOfSupportedInterfaceOrientations()
                target.requestGeometryUpdate(.iOS(interfaceOrientations: requested)) { [weak self] _ in
                    Task { @MainActor in self?.finish(identifier, error: .unavailable) }
                }
            }
        } onCancel: {
            Task { @MainActor [weak self] in self?.finish(identifier, error: .cancelled) }
        }
    }

    private func finish(_ identifier: UUID, error: PluginError) {
        guard let pending, pending.0 == identifier else { return }
        self.pending = nil
        for observer in observers { NotificationCenter.default.removeObserver(observer) }
        observers.removeAll()
        if let ownerKey { Self.owners.removeValue(forKey: ownerKey) }
        ownerKey = nil
        context.orientationMask = original
        original = nil
        controller?.setNeedsUpdateOfSupportedInterfaceOrientations()
        if let scene, scene.activationState == .foregroundActive {
            scene.requestGeometryUpdate(.iOS(interfaceOrientations: originalEffective)) { error in
                NSLog("LeptosKit orientation restoration rejected: %@", error.localizedDescription)
            }
        }
        controller = nil
        scene = nil
        pending.1.resume(throwing: error)
    }
    #endif
}

enum OrientationPolicy: UInt32 {
    case portrait, landscape, portraitPrimary, portraitSecondary, landscapePrimary, landscapeSecondary, any

    static func decode(_ bytes: UnsafeRawBufferPointer) throws -> Self {
        guard bytes.count == 4,
              let policy = Self(rawValue: UInt32(littleEndian: bytes.loadUnaligned(as: UInt32.self))) else {
            throw PluginError.invalidArgument
        }
        return policy
    }

    #if canImport(UIKit)
    var mask: UIInterfaceOrientationMask {
        switch self {
        case .portrait: [.portrait, .portraitUpsideDown]
        case .landscape: .landscape
        case .portraitPrimary: .portrait
        case .portraitSecondary: .portraitUpsideDown
        case .landscapePrimary: .landscapeRight
        case .landscapeSecondary: .landscapeLeft
        case .any: .all
        }
    }
    #endif
}
