//! GTK startup and event delivery. Application screens live in
//! file_routes_example_app.

#[cfg(target_os = "linux")]
mod runtime;

#[cfg(target_os = "linux")]
mod generated {
	include!(concat!(
		env!("CARGO_MANIFEST_DIR"),
		"/../.leptoskit/linux/registrant.rs"
	));
}

#[cfg(target_os = "linux")]
mod application {
	use std::cell::Cell;
	use std::cell::RefCell;
	use std::rc::Rc;
	use std::rc::Weak;
	use std::time::Duration;

	use leptoskit::gtk::GtkBackend;
	use leptoskit::gtk::gtk;
	use leptoskit::gtk::gtk::prelude::*;
	use leptoskit::native::RiveEnvironment;
	use leptoskit::wasm::WasmHost;

	use super::runtime;

	type Host = Rc<RefCell<WasmHost<GtkBackend>>>;
	type Failure = Rc<RefCell<Option<Box<dyn std::error::Error>>>>;
	const INITIAL_WIDTH: i32 = 800;
	const INITIAL_HEIGHT: i32 = 600;

	pub(super) fn run() -> runtime::AppResult<()> {
		gtk::init()?;
		let wasm = runtime::guest("linux")?;
		let mut host = WasmHost::new(GtkBackend::new(), &wasm)?;
		super::generated::configure(&mut host);
		host.start(INITIAL_WIDTH as f32, INITIAL_HEIGHT as f32)?;

		let host = Rc::new(RefCell::new(host));
		let failure = Rc::new(RefCell::new(None));
		let window = host.borrow().surface().backend().create_window(
			"file_routes_example",
			INITIAL_WIDTH,
			INITIAL_HEIGHT,
		);
		let frames = Rc::new(RiveFrames {
			host: Rc::downgrade(&host),
			window: window.downgrade(),
			failure: Rc::clone(&failure),
			callback: RefCell::new(None),
			previous: Cell::new(None),
			deadline: Cell::new(0),
		});
		install_events(&host, &window, &failure, &frames);
		install_viewport(&host, &window, &frames);
		let (settings, motion_listener) = install_rive_environment(&window, &frames);
		frames.request();
		leptoskit::gtk::run_application();
		frames.stop();
		settings.disconnect(motion_listener);

		if let Some(error) = failure.take() {
			return Err(error);
		}

		Ok(())
	}

	fn install_events(
		host: &Host,
		window: &gtk::Window,
		failure: &Failure,
		frames: &Rc<RiveFrames>,
	) {
		let weak = Rc::downgrade(host);
		let weak_window = window.downgrade();
		let event_failure = Rc::clone(failure);
		let frames = Rc::clone(frames);
		host.borrow()
			.surface()
			.backend()
			.set_event_listener(move |node, event| {
				if event_failure.borrow().is_some() {
					return;
				}

				let Some(host) = weak.upgrade() else {
					return;
				};
				let result = host.borrow_mut().dispatch(node, &event);

				if let Err(error) = result {
					event_failure.replace(Some(error.into()));

					close_window(&weak_window);
				} else {
					frames.request();
				}
			});
	}

	fn close_window(window: &gtk::glib::WeakRef<gtk::Window>) {
		if let Some(window) = window.upgrade() {
			window.close();
		}
	}

	fn install_viewport(host: &Host, window: &gtk::Window, frames: &Rc<RiveFrames>) {
		let weak = Rc::downgrade(host);
		let resized = Rc::clone(frames);
		host.borrow()
			.surface()
			.backend()
			.set_resize_listener(move |width, height| {
				if let Some(host) = weak.upgrade() {
					host.borrow_mut().set_viewport(width, height);
					resized.request();
				}
			});
		let weak = Rc::downgrade(host);
		let frames = Rc::clone(frames);
		window.connect_scale_factor_notify(move |_| {
			if let Some(host) = weak.upgrade() {
				host.borrow_mut().refresh_text_measurer();
				frames.request();
			}
		});
	}

	/// The GTK frame clock drives visible scenes, with no polling source while
	/// idle.
	struct RiveFrames {
		host: Weak<RefCell<WasmHost<GtkBackend>>>,
		window: gtk::glib::WeakRef<gtk::Window>,
		failure: Failure,
		callback: RefCell<Option<gtk::TickCallbackId>>,
		previous: Cell<Option<i64>>,
		deadline: Cell<i64>,
	}

	impl RiveFrames {
		fn stop(&self) {
			if let Some(callback) = self.callback.take() {
				callback.remove();
			}
			self.previous.set(None);
			self.deadline.set(0);
		}

		fn request(self: &Rc<Self>) {
			let Some(window) = self.window.upgrade() else {
				self.stop();
				return;
			};
			if !window.is_mapped() || !window.is_active() || self.failure.borrow().is_some() {
				self.stop();
				return;
			}
			if self.callback.borrow().is_some() {
				return;
			}
			let Some(host) = self.host.upgrade() else {
				return;
			};
			if !host.borrow().has_rive_players() {
				return;
			}
			let weak = Rc::downgrade(self);
			let callback = window.add_tick_callback(move |window, clock| {
				let Some(owner) = weak.upgrade() else {
					return gtk::glib::ControlFlow::Break;
				};
				let Some(host) = owner.host.upgrade() else {
					return gtk::glib::ControlFlow::Break;
				};
				let now = clock.frame_time();
				// High-refresh displays still rasterize at most 60
				// frames/second.
				if now + 500 < owner.deadline.get() {
					return gtk::glib::ControlFlow::Continue;
				}
				owner.deadline.set(if owner.deadline.get() == 0 {
					now + 16_667
				} else {
					(owner.deadline.get() + 16_667).max(now + 1_000)
				});
				let elapsed = owner
					.previous
					.replace(Some(now))
					.map_or(Duration::ZERO, |last| {
						Duration::from_micros(now.saturating_sub(last).max(0) as u64)
					});
				let result = host.borrow_mut().tick_rive(
					elapsed,
					RiveEnvironment {
						scale: window.scale_factor() as f32,
						active: window.is_mapped() && window.is_active(),
						reduced_motion: !window.settings().is_gtk_enable_animations(),
					},
				);
				match result {
					Ok(true) => gtk::glib::ControlFlow::Continue,
					result => {
						owner.callback.take();
						owner.previous.set(None);
						owner.deadline.set(0);
						if let Err(error) = result {
							owner.failure.replace(Some(error.into()));
							window.close();
						}
						gtk::glib::ControlFlow::Break
					}
				}
			});
			self.callback.replace(Some(callback));
		}
	}

	fn install_rive_environment(
		window: &gtk::Window,
		frames: &Rc<RiveFrames>,
	) -> (gtk::Settings, gtk::glib::SignalHandlerId) {
		let owner = Rc::clone(frames);
		window.connect_is_active_notify(move |_| owner.request());
		let owner = Rc::clone(frames);
		window.connect_map(move |_| owner.request());
		let owner = Rc::clone(frames);
		window.connect_unmap(move |_| owner.stop());
		let owner = Rc::downgrade(frames);
		let settings = window.settings();
		let listener = settings.connect_gtk_enable_animations_notify(move |_| {
			if let Some(owner) = owner.upgrade() {
				owner.request();
			}
		});
		(settings, listener)
	}
}

#[cfg(target_os = "linux")]
fn main() -> runtime::AppResult<()> {
	application::run()
}

#[cfg(not(target_os = "linux"))]
fn main() {
	eprintln!("build the Linux runner on Linux with `leptoskit build --platform linux`");
	std::process::exit(1);
}
