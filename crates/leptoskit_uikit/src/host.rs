//! `UIApplication` and window-scene hosting without a second rendering tree.

use std::cell::RefCell;

use leptoskit_yoga::NodeId;
use leptoskit_yoga::Rect;
use objc2::ClassType;
use objc2::DefinedClass;
use objc2::MainThreadMarker;
use objc2::MainThreadOnly;
use objc2::define_class;
use objc2::msg_send;
use objc2::rc::Retained;
use objc2::runtime::NSObject;
use objc2::runtime::NSObjectProtocol;
use objc2_foundation::NSString;
use objc2_ui_kit::UIApplication;
use objc2_ui_kit::UIApplicationDelegate;
use objc2_ui_kit::UIScene;
use objc2_ui_kit::UISceneConfiguration;
use objc2_ui_kit::UISceneConnectionOptions;
use objc2_ui_kit::UISceneDelegate;
use objc2_ui_kit::UISceneSession;
use objc2_ui_kit::UIViewController;
use objc2_ui_kit::UIWindow;
use objc2_ui_kit::UIWindowScene;
use objc2_ui_kit::UIWindowSceneDelegate;

use crate::UIKitBackend;

type Launch = Box<dyn FnMut(MainThreadMarker, &UIWindowScene) -> Retained<UIWindow>>;
thread_local! { static LAUNCH: RefCell<Option<Launch>> = const { RefCell::new(None) }; }

struct HostState {
	window: RefCell<Option<Retained<UIWindow>>>,
}

define_class!(
	// SAFETY: The application owns this NSObject delegate and its lifecycle.
	#[unsafe(super(NSObject))]
	#[thread_kind = MainThreadOnly]
	struct HostDelegate;
	unsafe impl NSObjectProtocol for HostDelegate {}
	unsafe impl UIApplicationDelegate for HostDelegate {
		#[unsafe(method_id(application:configurationForConnectingSceneSession:options:))]
		fn configuration(
			&self,
			_application: &UIApplication,
			session: &UISceneSession,
			_options: &UISceneConnectionOptions,
		) -> Retained<UISceneConfiguration> {
			let configuration = UISceneConfiguration::initWithName_sessionRole(
				UISceneConfiguration::alloc(self.mtm()),
				Some(&NSString::from_str("leptoskit")),
				&session.role(),
			);
			// SAFETY: This class implements UIWindowSceneDelegate and its
			// window.
			unsafe {
				configuration.setDelegateClass(Some(SceneDelegate::class()));
			}
			configuration
		}
	}
);

define_class!(
	// SAFETY: The scene owns this initialized NSObject delegate and its window.
	#[unsafe(super(NSObject))]
	#[thread_kind = MainThreadOnly]
	#[ivars = HostState]
	struct SceneDelegate;
	impl SceneDelegate {
		#[unsafe(method_id(init))]
		fn init(this: objc2::rc::Allocated<Self>) -> Retained<Self> {
			let this = this.set_ivars(HostState { window: RefCell::new(None) });
			// SAFETY: NSObject's designated initializer initializes this instance.
			unsafe { msg_send![super(this), init] }
		}
	}
	unsafe impl NSObjectProtocol for SceneDelegate {}
	unsafe impl UISceneDelegate for SceneDelegate {
		#[unsafe(method(scene:willConnectToSession:options:))]
		fn connect(&self, scene: &UIScene, _session: &UISceneSession, _options: &UISceneConnectionOptions) {
			let Some(scene) = scene.downcast_ref::<UIWindowScene>() else { return; };
			let window = LAUNCH.with(|launch| launch.borrow_mut().as_mut().expect("run_application installs its launch callback")(self.mtm(), scene));
			window.makeKeyAndVisible();
			self.ivars().window.replace(Some(window));
		}

		#[unsafe(method(sceneDidDisconnect:))]
		fn disconnect(&self, _scene: &UIScene) {
			if let Some(window) = self.ivars().window.borrow_mut().take() {
				window.setHidden(true);
				window.setRootViewController(None);
			}
		}
	}
	unsafe impl UIWindowSceneDelegate for SceneDelegate {
		#[unsafe(method_id(window))]
		fn window(&self) -> Option<Retained<UIWindow>> { self.ivars().window.borrow().clone() }
		#[unsafe(method(setWindow:))]
		fn set_window(&self, window: Option<&UIWindow>) { self.ivars().window.replace(window.map(Message::retain)); }
	}
);

impl UIKitBackend {
	/// Installs a live node as the content view of an existing controller.
	///
	/// The application retains the surface and updates its viewport when the
	/// controller's bounds change. Replacing the root requires mounting it
	/// again. Returns false if the node no longer exists.
	pub fn mount(&self, root: NodeId, controller: &UIViewController) -> bool {
		let _events = self.suppress_events();
		let Some(view) = self.view(root) else {
			return false;
		};
		let container = controller
			.view()
			.expect("UIKit controller loads its root view");

		if view
			.superview()
			.as_deref()
			.is_some_and(|parent| std::ptr::eq(parent, &raw const *container))
		{
			return true;
		}

		for child in &container.subviews() {
			child.removeFromSuperview();
		}

		container.addSubview(view);
		true
	}

	/// Creates a window and controller showing a root within the connected
	/// scene.
	///
	/// Keep the surface alive and forward subsequent viewport changes to it.
	pub fn open_window(&self, root: NodeId, scene: &UIWindowScene) -> Option<Retained<UIWindow>> {
		let view = self.view(root)?;
		let window =
			UIWindow::initWithWindowScene(UIWindow::alloc(self.main_thread_marker()), scene);
		let controller = UIViewController::new(self.main_thread_marker());
		let _root = view;
		self.mount(root, &controller);
		window.setRootViewController(Some(&controller));
		Some(window)
	}
}

/// Runs a UIKit application with the modern window-scene lifecycle.
///
/// Invoke once from the executable's main thread. The callback creates one
/// window and retained surface per scene. Its bundle must opt into scenes with
/// `UIApplicationSceneManifest`, as the native probe's Info.plist does.
/// The callback is called again if a disconnected scene reconnects.
pub fn run_application(
	mtm: MainThreadMarker,
	launch: impl FnMut(MainThreadMarker, &UIWindowScene) -> Retained<UIWindow> + 'static,
) -> ! {
	LAUNCH.with(|slot| {
		assert!(
			slot.borrow().is_none(),
			"one UIKit application launch callback"
		);
		slot.replace(Some(Box::new(launch)));
	});
	UIApplication::main(
		None,
		Some(&NSString::from_class(HostDelegate::class())),
		mtm,
	)
}

use std::cell::Cell;
use std::rc::Rc;

use leptoskit_native::Element;
use leptoskit_native::NativeSurface;
use objc2::Message;
use objc2::rc::Weak;
use objc2_core_foundation::CGSize;
use objc2_ui_kit::UIEdgeInsets;

struct SurfaceState {
	surface: RefCell<NativeSurface<UIKitBackend>>,
	render: Box<dyn Fn() -> Element>,
	controller: RefCell<Option<Weak<SurfaceController>>>,
	/// The bounds and safe-area insets of the latest render.
	layout: Cell<(CGSize, UIEdgeInsets)>,
}

impl SurfaceState {
	fn render(&self, controller: &UIViewController, size: CGSize) {
		let insets = controller
			.view()
			.expect("UIKit controller loads its root view")
			.safeAreaInsets();
		self.layout.set((size, insets));
		let tree = (self.render)();
		let mut surface = self.surface.borrow_mut();
		surface.set_viewport(size.width as f32, size.height as f32);
		surface.set_safe_area(Rect::all_edges(
			insets.top as f32,
			insets.right as f32,
			insets.bottom as f32,
			insets.left as f32,
		));
		surface.set_root(&tree);

		if let Some(root) = surface.root() {
			surface.backend().mount(root, controller);
		}
	}

	fn refresh(&self) {
		let controller = self.controller.borrow().as_ref().and_then(Weak::load);

		if let Some(controller) = controller {
			self.render(
				&controller,
				controller
					.view()
					.expect("UIKit controller loads its root view")
					.bounds()
					.size,
			);
		}
	}
}

struct ControllerState {
	host: Rc<SurfaceState>,
}

impl Drop for ControllerState {
	fn drop(&mut self) {
		self.host.surface.borrow().backend().pause_animations();
	}
}

define_class!(
	// SAFETY: UIViewController's designated initializer is used. UIKit owns the
	// outer view, and the existing NativeSurface owns every shared widget.
	#[unsafe(super(UIViewController, objc2_ui_kit::UIResponder, NSObject))]
	#[thread_kind = MainThreadOnly]
	#[ivars = ControllerState]
	struct SurfaceController;
	impl SurfaceController {
		#[unsafe(method(viewDidLayoutSubviews))]
		fn layout(&self) {
			let _keep_alive = self.retain();
			// SAFETY: UIViewController implements this lifecycle selector.
			unsafe { let _: () = msg_send![super(self), viewDidLayoutSubviews]; }
			// Rotation, resizing, and status bar changes all lay out again.
			let view = self.view().expect("UIKit controller loads its root view");
			let size = view.bounds().size;
			if (size, view.safeAreaInsets()) != self.ivars().host.layout.get() { self.ivars().host.render(self, size); }
		}
	}
	unsafe impl NSObjectProtocol for SurfaceController {}
);

impl SurfaceController {
	fn new(mtm: MainThreadMarker, host: Rc<SurfaceState>) -> Retained<Self> {
		let this = Self::alloc(mtm).set_ivars(ControllerState { host });
		// SAFETY: A nil nib name/bundle initializes a programmatically hosted
		// UIViewController through its designated initializer.
		unsafe {
			msg_send![super(this), initWithNibName: Option::<&NSString>::None, bundle: Option::<&objc2_foundation::NSBundle>::None]
		}
	}
}

/// A retained shared surface hosted in a stable UIKit view controller.
///
/// Element handlers update application state, then `render` returns its latest
/// tree. The controller keeps this host alive, forwards layout/rotation bounds
/// and safe-area insets (for `*-safe` padding), and remounts replacement roots
/// without changing Yoga's chosen root frame. Its listener and controller
/// references are weak, so closing the window releases the surface and stops
/// its native animations.
#[derive(Clone)]
pub struct UIKitHost {
	state: Rc<SurfaceState>,
}

impl UIKitHost {
	/// Creates one native surface over an application render function.
	pub fn new(mtm: MainThreadMarker, render: impl Fn() -> Element + 'static) -> Self {
		let state = Rc::new(SurfaceState {
			surface: RefCell::new(NativeSurface::new(UIKitBackend::new(mtm))),
			render: Box::new(render),
			controller: RefCell::new(None),
			layout: Cell::new((
				CGSize::ZERO,
				UIEdgeInsets {
					top: 0.0,
					left: 0.0,
					bottom: 0.0,
					right: 0.0,
				},
			)),
		});
		let weak = Rc::downgrade(&state);
		state
			.surface
			.borrow_mut()
			.backend_mut()
			.set_event_listener(Some(Rc::new(move |node, event| {
				let Some(state) = weak.upgrade() else {
					return;
				};
				state.surface.borrow().dispatch_event(node, event);
				state.refresh();
			})));
		Self { state }
	}

	/// Creates one scene window. Its controller retains this host's surface.
	///
	/// A host owns one window/controller pair; create another host for a second
	/// scene. Subsequent root replacement and viewport changes are automatic.
	pub fn open_window(&self, scene: &UIWindowScene) -> Retained<UIWindow> {
		assert!(
			self.state
				.controller
				.borrow()
				.as_ref()
				.and_then(Weak::load)
				.is_none(),
			"one window per UIKitHost"
		);
		let mtm = self.state.surface.borrow().backend().main_thread_marker();
		self.state.surface.borrow().backend().resume_animations();
		let controller = SurfaceController::new(mtm, Rc::clone(&self.state));
		self.state.controller.replace(Some(Weak::new(&controller)));
		let window = UIWindow::initWithWindowScene(UIWindow::alloc(mtm), scene);
		self.state.render(&controller, window.bounds().size);
		window.setRootViewController(Some(&controller));

		window
	}

	/// Applies the application's explicit scheme to current and future widgets.
	pub fn set_dark_mode(&self, dark: bool) {
		self.state.surface.borrow_mut().set_dark_mode(dark);
		self.state.refresh();
	}

	/// Reconciles the latest application state without an input event.
	pub fn refresh(&self) {
		self.state.refresh();
	}
}
