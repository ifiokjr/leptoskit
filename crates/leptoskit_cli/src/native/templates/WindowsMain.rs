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
mod application {
	use std::cell::Cell;
	use std::cell::RefCell;
	use std::rc::Rc;
	use std::time::Duration;
	use std::time::Instant;

	use leptoskit::native::RiveEnvironment;
	use leptoskit::wasm::WasmHost;
	use leptoskit::windows::WindowsBackend;
	use leptoskit::windows::WindowsWindow;
	use leptoskit::yoga::NodeId;

	use super::runtime;

	type Host = Rc<RefCell<WasmHost<WindowsBackend>>>;
	type Mounted = Rc<Cell<Option<NodeId>>>;
	type Failure = Rc<RefCell<Option<Box<dyn std::error::Error>>>>;
	const INITIAL_WIDTH: i32 = 800;
	const INITIAL_HEIGHT: i32 = 600;
	/// How often a development session checks for a rebuilt guest.
	const DEVELOPMENT_POLL: std::time::Duration = std::time::Duration::from_millis(100);

	pub(super) fn run() -> runtime::AppResult<()> {
		let window = Rc::new(WindowsWindow::new(
			"__NAME__",
			INITIAL_WIDTH,
			INITIAL_HEIGHT,
		)?);
		let mut backend = WindowsBackend::new()?;
		backend.set_dpi(window.dpi());
		let wasm = runtime::guest("windows")?;
		let mut app = WasmHost::new(backend, &wasm)?;
		super::generated::configure(&mut app);
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
		install_events(&host, &window, &mounted, &failure);
		install_viewport(&host, &window, &mounted, &failure);
		let guests = leptoskit::codepush::dev::follow_environment(&wasm);
		window.show();
		run_frames(&host, &window, &mounted, guests.as_ref())?;

		if let Some(error) = failure.take() {
			return Err(error);
		}

		Ok(())
	}

	fn reload(
		host: &mut WasmHost<WindowsBackend>,
		window: &WindowsWindow,
		wasm: &[u8],
	) -> runtime::AppResult<()> {
		host.replace_guest(wasm)?;
		super::generated::configure(host);
		let (width, height) = window.client_size()?;
		host.start(width, height)?;
		eprintln!("leptoskit dev: reloaded");
		Ok(())
	}

	/// Sleep on native messages when idle. Active scenes use a capped 60 Hz
	/// deadline; no timer survives minimization, focus loss, or reduced motion.
	/// Development guests share this loop, without keeping a frame timer alive.
	fn run_frames(
		host: &Host,
		window: &WindowsWindow,
		mounted: &Mounted,
		guests: Option<&std::sync::mpsc::Receiver<Vec<u8>>>,
	) -> runtime::AppResult<()> {
		let interval = Duration::from_nanos(1_000_000_000 / 60);
		let mut deadline = Instant::now();
		let mut previous = None;
		let mut pending = false;
		while window.is_open() {
			window.pump_messages();
			if !window.is_open() {
				break;
			}
			if let Some(wasm) = guests.and_then(|guests| guests.try_iter().last()) {
				let result = reload(&mut host.borrow_mut(), window, &wasm);
				let result = result.and_then(|()| remount(window, &host.borrow(), mounted));
				if let Err(error) = result {
					eprintln!("leptoskit dev: {error}");
				} else {
					previous = None;
					pending = false;
				}
			}
			let now = Instant::now();
			if !pending || now >= deadline {
				let elapsed =
					previous.map_or(Duration::ZERO, |last| now.saturating_duration_since(last));
				pending = host.borrow_mut().tick_rive(
					elapsed,
					RiveEnvironment {
						scale: window.dpi() as f32 / 96.0,
						active: window.is_active(),
						reduced_motion: !window.animations_enabled()?,
					},
				)?;
				previous = pending.then_some(now);
				deadline = now + interval;
			}
			let wait = pending.then(|| deadline.saturating_duration_since(Instant::now()));
			let wait = if guests.is_some() {
				Some(wait.map_or(DEVELOPMENT_POLL, |wait| wait.min(DEVELOPMENT_POLL)))
			} else {
				wait
			};
			window.wait_for_messages(wait)?;
		}
		Ok(())
	}

	fn install_events(
		host: &Host,
		window: &Rc<WindowsWindow>,
		mounted: &Mounted,
		failure: &Failure,
	) {
		let (weak, weak_window) = (Rc::downgrade(host), Rc::downgrade(window));
		let event_mounted = Rc::clone(mounted);
		let event_failure = Rc::clone(failure);
		let listener = Rc::new(move |node, event| {
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
				close_failed(&window);
			}
		});
		host.borrow_mut()
			.update_backend(|backend| backend.set_event_listener(listener));
	}

	fn install_viewport(
		host: &Host,
		window: &Rc<WindowsWindow>,
		mounted: &Mounted,
		failure: &Failure,
	) {
		let mounted = Rc::clone(mounted);
		let (weak, weak_window) = (Rc::downgrade(host), Rc::downgrade(window));
		let resize_failure = Rc::clone(failure);
		let previous_dpi = Cell::new(window.dpi());
		window.set_resize_listener(Rc::new(move |width, height, dpi| {
			let (Some(host), Some(window)) = (weak.upgrade(), weak_window.upgrade()) else {
				return;
			};

			if !window.is_open() {
				return;
			}

			update_viewport(&host, &previous_dpi, (width, height, dpi));

			if let Err(error) = remount(&window, &host.borrow(), &mounted) {
				resize_failure.replace(Some(error));
				close_failed(&window);
			}
		}));
	}

	fn update_viewport(host: &Host, previous_dpi: &Cell<u32>, size: (f32, f32, u32)) {
		let (width, height, dpi) = size;
		let mut host = host.borrow_mut();

		if previous_dpi.replace(dpi) != dpi {
			host.update_backend(|backend| backend.set_dpi(dpi));
		}

		host.set_viewport(width, height);
	}

	fn close_failed(window: &WindowsWindow) {
		if let Err(error) = window.close() {
			eprintln!("could not close the failed application: {error}");
			std::process::exit(1);
		}
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
}

#[cfg(target_os = "windows")]
fn main() -> runtime::AppResult<()> {
	application::run()
}

#[cfg(not(target_os = "windows"))]
fn main() {
	eprintln!("build the Windows runner on Windows with `leptoskit build --platform windows`");
	std::process::exit(1);
}
