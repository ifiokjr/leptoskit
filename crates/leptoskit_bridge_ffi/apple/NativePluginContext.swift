import CLeptosKitBridge
import Foundation
#if canImport(UIKit)
    import UIKit
    public typealias NativePresentationOwner = UIViewController
#elseif canImport(AppKit)
    import AppKit
    public typealias NativePresentationOwner = NSViewController
#endif

#if canImport(UIKit) || canImport(AppKit)
    /// The owning app's presentation context. It never discovers global windows
    /// and does not extend the controller's lifetime. Plugins may retain context.
    @MainActor
    public final class NativePluginContext {
        private weak var owner: NativePresentationOwner?

        #if canImport(UIKit)
            /// An optional orientation restriction observed by the owning controller.
            /// The plugin requests scene geometry and orientation updates explicitly.
            public var orientationMask: UIInterfaceOrientationMask?
        #endif

        public init(owner: NativePresentationOwner) {
            self.owner = owner
        }

        /// Returns this app's attached presentation owner, without loading its view.
        /// A detached, closed, or inactive iOS scene cannot present a native dialog.
        public func requirePresentationOwner() throws -> NativePresentationOwner {
            guard let owner, owner.isViewLoaded, let window = owner.view.window else {
                throw PluginError.unavailable
            }

            #if canImport(UIKit)

                guard window.windowScene?.activationState == .foregroundActive else {
                    throw PluginError.unavailable
                }

            #else

                guard window.isVisible else { throw PluginError.unavailable }
            #endif

            return owner
        }

        /// Returns this app's owner while its view is in a window, without loading
        /// the view. Plugins that only observe the window, such as its hinge, use
        /// this: unlike presenting, observing needs no foreground-active scene, and
        /// the guest's first calls arrive while the scene is still activating.
        public func requireAttachedOwner() throws -> NativePresentationOwner {
            guard let owner, owner.isViewLoaded, owner.view.window != nil else {
                throw PluginError.unavailable
            }

            return owner
        }

        /// Called after plugin close callbacks, before the app runtime is destroyed.
        public func invalidate() {
            owner = nil
        }

        /// Installs a retained weak-owner bridge into `context` so Rust plugins
        /// can resolve the live presentation or attached owner on the main
        /// thread without extending the controller's lifetime.
        public func retainForAppContext(_ context: inout lk_app_context) {
            typealias OwnerResolver = @convention(c) (UnsafeMutableRawPointer?, Int32) -> UnsafeMutableRawPointer?
            let resolver: OwnerResolver = { rawContext, kind in
                guard let rawContext, Thread.isMainThread else { return nil }
                let contextAddress = UInt(bitPattern: rawContext)
                let ownerAddress: UInt = MainActor.assumeIsolated {
                    guard let ptr = UnsafeMutableRawPointer(bitPattern: contextAddress) else {
                        return 0
                    }
                    let pluginContext = Unmanaged<NativePluginContext>.fromOpaque(ptr)
                        .takeUnretainedValue()
                    let controller: NativePresentationOwner?
                    switch kind {
                    case 0:
                        controller = try? pluginContext.requireAttachedOwner()
                    case 1:
                        controller = try? pluginContext.requirePresentationOwner()
                    default:
                        controller = nil
                    }
                    guard let controller else { return 0 }
                    return UInt(bitPattern: Unmanaged.passUnretained(controller).toOpaque())
                }
                return UnsafeMutableRawPointer(bitPattern: ownerAddress)
            }
            context.platform_vm = unsafeBitCast(resolver, to: UnsafeMutableRawPointer.self)
            context.platform_context = Unmanaged.passRetained(self).toOpaque()
            context.release_platform_context = { _, rawContext in
                guard let rawContext else { return }
                let address = UInt(bitPattern: rawContext)
                if Thread.isMainThread {
                    MainActor.assumeIsolated {
                        if let ptr = UnsafeMutableRawPointer(bitPattern: address) {
                            Unmanaged<NativePluginContext>.fromOpaque(ptr).release()
                        }
                    }
                } else {
                    DispatchQueue.main.async {
                        if let ptr = UnsafeMutableRawPointer(bitPattern: address) {
                            MainActor.assumeIsolated {
                                Unmanaged<NativePluginContext>.fromOpaque(ptr).release()
                            }
                        }
                    }
                }
            }
        }
    }
#endif
