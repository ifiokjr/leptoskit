//! Win32 startup and event delivery. Application screens live in __NAME___app.

#[cfg(target_os = "windows")]
mod runtime;

#[cfg(target_os = "windows")]
mod generated {
	include!(concat!(
		env!("CARGO_MANIFEST_DIR"),
		"/../.leptoskit/windows/registrant.rs"
	));
}

#[cfg(target_os = "windows")]
fn main() -> runtime::AppResult<()> {
	use std::cell::Cell;
	use std::cell::RefCell;
	use std::rc::Rc;

	use leptoskit::wasm::WasmHost;
	use leptoskit::windows::WindowsBackend;
	use leptoskit::windows::WindowsWindow;

	let window = Rc::new(WindowsWindow::new("__NAME__", 800, 600)?);
	let mut backend = WindowsBackend::new()?;
	backend.set_dpi(window.dpi());
	let wasm = runtime::guest("windows")?;
	let mut app = WasmHost::new(backend, &wasm)?;
	generated::configure(&mut app);
	let (width, height) = window.client_size()?;
	app.start(width, height)?;
	let root = app
		.surface()
		.root()
		.ok_or("the application committed no root")?;
	window.mount(app.surface().backend(), root)?;
	let mounted = Rc::new(Cell::new(Some(root)));
	let host = Rc::new(RefCell::new(app));
	let failure = Rc::new(RefCell::new(None));
	let (weak, weak_window) = (Rc::downgrade(&host), Rc::downgrade(&window));
	let event_mounted = Rc::clone(&mounted);
	let event_failure = Rc::clone(&failure);
	host.borrow_mut().update_backend(|backend| {
		backend.set_event_listener(Rc::new(move |node, event| {
			let (Some(host), Some(window)) = (weak.upgrade(), weak_window.upgrade()) else {
				return;
			};

			if !window.is_open() {
				return;
			}

			let dispatch = host.borrow_mut().dispatch(node, &event);
			let result = dispatch
				.map(|_| ())
				.map_err(|error| -> Box<dyn std::error::Error> { error.into() })
				.and_then(|()| remount(&window, &host.borrow(), &event_mounted));

			if let Err(error) = result {
				event_failure.replace(Some(error));
				if let Err(error) = window.close() {
					eprintln!("could not close the failed application: {error}");
					std::process::exit(1);
				}
			}
		}));
	});
	let (weak, weak_window) = (Rc::downgrade(&host), Rc::downgrade(&window));
	let resize_failure = Rc::clone(&failure);
	let previous_dpi = Cell::new(window.dpi());
	window.set_resize_listener(Rc::new(move |width, height, dpi| {
		let (Some(host), Some(window)) = (weak.upgrade(), weak_window.upgrade()) else {
			return;
		};

		if !window.is_open() {
			return;
		}

		{
			let mut host = host.borrow_mut();

			if previous_dpi.replace(dpi) != dpi {
				host.update_backend(|backend| backend.set_dpi(dpi));
			}

			host.set_viewport(width, height);
		}

		if let Err(error) = remount(&window, &host.borrow(), &mounted) {
			resize_failure.replace(Some(error));
			if let Err(error) = window.close() {
				eprintln!("could not close the failed application: {error}");
				std::process::exit(1);
			}
		}
	}));
	window.show();
	window.run()?;

	if let Some(error) = failure.take() {
		return Err(error);
	}

	Ok(())
}

/// A guest may replace its root while responding to an event or viewport.
#[cfg(target_os = "windows")]
fn remount(
	window: &leptoskit::windows::WindowsWindow,
	host: &leptoskit::wasm::WasmHost<leptoskit::windows::WindowsBackend>,
	mounted: &std::cell::Cell<Option<leptoskit::yoga::NodeId>>,
) -> runtime::AppResult<()> {
	if !window.is_open() {
		return Ok(());
	}

	let root = host
		.surface()
		.root()
		.ok_or("the application committed no root")?;

	if mounted.get() != Some(root) {
		window.mount(host.surface().backend(), root)?;
		mounted.set(Some(root));
	}

	Ok(())
}

#[cfg(not(target_os = "windows"))]
fn main() {
	eprintln!("build the Windows runner on Windows with `leptoskit build --platform windows`");
	std::process::exit(1);
}
