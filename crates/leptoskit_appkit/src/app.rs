//! Showing a view tree in a real window and running the application.

use leptoskit_yoga::NodeId;
use objc2::MainThreadMarker;
use objc2::MainThreadOnly;
use objc2::define_class;
use objc2::msg_send;
use objc2::rc::Retained;
use objc2::runtime::NSObject;
use objc2::runtime::NSObjectProtocol;
use objc2::runtime::ProtocolObject;
use objc2_app_kit::NSApplication;
use objc2_app_kit::NSApplicationActivationPolicy;
use objc2_app_kit::NSApplicationDelegate;
use objc2_app_kit::NSBackingStoreType;
use objc2_app_kit::NSWindow;
use objc2_app_kit::NSWindowStyleMask;
use objc2_foundation::NSPoint;
use objc2_foundation::NSRect;
use objc2_foundation::NSString;

use crate::backend::AppKitBackend;

define_class!(
	// SAFETY:
	// - `NSObject` has no subclassing requirements.
	// - The class does not implement `Drop`.
	#[unsafe(super(NSObject))]
	#[thread_kind = MainThreadOnly]
	/// Quits the application when its last window closes.
	struct AppDelegate;

	unsafe impl NSObjectProtocol for AppDelegate {}

	unsafe impl NSApplicationDelegate for AppDelegate {
		#[unsafe(method(applicationShouldTerminateAfterLastWindowClosed:))]
		fn should_terminate_after_last_window_closed(&self, _sender: &NSApplication) -> bool {
			true
		}
	}
);

impl AppDelegate {
	fn new(mtm: MainThreadMarker) -> Retained<Self> {
		let this = Self::alloc(mtm).set_ivars(());
		// SAFETY: `init` is `NSObject`'s designated initializer.
		unsafe { msg_send![super(this), init] }
	}
}

impl AppKitBackend {
	/// Shows a node's view as the content of a new titled window, sized to
	/// the view's current frame, centered on screen and brought to the
	/// front.
	///
	/// The window is not resizable: the surface's viewport is fixed, and the
	/// window matches it. Keep the returned window alive for as long as it
	/// should stay open, then call [`run_application`] to hand control to
	/// AppKit.
	///
	/// Returns `None` when `root` is not a live widget of this backend.
	pub fn open_window(&self, root: NodeId, title: &str) -> Option<Retained<NSWindow>> {
		let view = self.view(root)?;
		let size = view.frame().size;
		let mtm = self.main_thread_marker();
		// SAFETY: a buffered, non-deferred window with a valid content
		// rectangle and style mask is the standard configuration; the window
		// is released by `Retained`, not on close, as set below.
		let window = unsafe {
			NSWindow::initWithContentRect_styleMask_backing_defer(
				NSWindow::alloc(mtm),
				NSRect::new(NSPoint::ZERO, size),
				NSWindowStyleMask::Titled
					| NSWindowStyleMask::Closable
					| NSWindowStyleMask::Miniaturizable,
				NSBackingStoreType::Buffered,
				false,
			)
		};
		// SAFETY: the window is owned by the returned `Retained`, so AppKit
		// must not also release it when it closes.
		unsafe { window.setReleasedWhenClosed(false) };
		window.setTitle(&NSString::from_str(title));
		window.setContentView(Some(view));
		window.center();

		window.makeKeyAndOrderFront(None);
		Some(window)
	}
}

/// Runs the AppKit event loop as a regular, foreground application until its
/// last window closes, at which point the process exits.
///
/// Open at least one window with [`AppKitBackend::open_window`] first.
pub fn run_application(mtm: MainThreadMarker) {
	let app = NSApplication::sharedApplication(mtm);
	app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
	// The application holds its delegate weakly; this binding keeps it alive
	// for the whole run, which only ends with the process.
	let delegate = AppDelegate::new(mtm);
	app.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
	#[allow(deprecated)]
	app.activateIgnoringOtherApps(true);
	app.run();
}
