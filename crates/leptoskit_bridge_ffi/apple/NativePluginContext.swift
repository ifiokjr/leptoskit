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

        /// Called after plugin close callbacks, before the app runtime is destroyed.
        public func invalidate() {
            owner = nil
        }
    }
#endif
