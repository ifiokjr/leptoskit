//! GTK startup and event delivery. Application screens live in tier_app.

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
	use std::cell::RefCell;
	use std::rc::Rc;

	use leptoskit::gtk::GtkBackend;
	use leptoskit::gtk::gtk;
	use leptoskit::gtk::gtk::prelude::*;
	use leptoskit::wasm::WasmError;
	use leptoskit::wasm::WasmHost;

	use super::runtime;

	type Host = Rc<RefCell<WasmHost<GtkBackend>>>;
	type Failure = Rc<RefCell<Option<WasmError>>>;
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
		let window =
			host.borrow()
				.surface()
				.backend()
				.create_window("tier", INITIAL_WIDTH, INITIAL_HEIGHT);
		install_events(&host, &window, &failure);
		install_viewport(&host, &window);
		leptoskit::gtk::run_application();

		if let Some(error) = failure.take() {
			return Err(error.into());
		}

		Ok(())
	}

	fn install_events(host: &Host, window: &gtk::Window, failure: &Failure) {
		let weak = Rc::downgrade(host);
		let weak_window = window.downgrade();
		let event_failure = Rc::clone(failure);
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
					event_failure.replace(Some(error));

					close_window(&weak_window);
				}
			});
	}

	fn close_window(window: &gtk::glib::WeakRef<gtk::Window>) {
		if let Some(window) = window.upgrade() {
			window.close();
		}
	}

	fn install_viewport(host: &Host, window: &gtk::Window) {
		let weak = Rc::downgrade(host);
		host.borrow()
			.surface()
			.backend()
			.set_resize_listener(move |width, height| {
				if let Some(host) = weak.upgrade() {
					host.borrow_mut().set_viewport(width, height);
				}
			});
		let weak = Rc::downgrade(host);
		window.connect_scale_factor_notify(move |_| {
			if let Some(host) = weak.upgrade() {
				host.borrow_mut().refresh_text_measurer();
			}
		});
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
