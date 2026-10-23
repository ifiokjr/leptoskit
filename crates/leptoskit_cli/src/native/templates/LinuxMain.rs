//! GTK startup and event delivery. Application screens live in __NAME___app.

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
fn main() -> runtime::AppResult<()> {
	use std::cell::RefCell;
	use std::rc::Rc;

	use leptoskit::gtk::GtkBackend;
	use leptoskit::gtk::gtk;
	use leptoskit::gtk::gtk::prelude::*;
	use leptoskit::wasm::WasmHost;

	gtk::init()?;
	let wasm = runtime::guest("linux")?;
	let mut host = WasmHost::new(GtkBackend::new(), &wasm)?;
	generated::configure(&mut host);
	host.start(800.0, 600.0)?;
	let host = Rc::new(RefCell::new(host));
	let failure = Rc::new(RefCell::new(None));
	let window = host
		.borrow()
		.surface()
		.backend()
		.create_window("__NAME__", 800, 600);
	let weak = Rc::downgrade(&host);
	let weak_window = window.downgrade();
	let event_failure = Rc::clone(&failure);
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

				if let Some(window) = weak_window.upgrade() {
					window.close();
				}
			}
		});
	let weak = Rc::downgrade(&host);
	host.borrow()
		.surface()
		.backend()
		.set_resize_listener(move |width, height| {
			if let Some(host) = weak.upgrade() {
				host.borrow_mut().set_viewport(width, height);
			}
		});
	let weak = Rc::downgrade(&host);
	window.connect_scale_factor_notify(move |_| {
		if let Some(host) = weak.upgrade() {
			host.borrow_mut().refresh_text_measurer();
		}
	});
	leptoskit::gtk::run_application();

	if let Some(error) = failure.take() {
		return Err(error.into());
	}

	Ok(())
}

#[cfg(not(target_os = "linux"))]
fn main() {
	eprintln!("build the Linux runner on Linux with `leptoskit build --platform linux`");
	std::process::exit(1);
}
