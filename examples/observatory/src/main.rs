//! Opens the observatory in a macOS window, or prints what it would show.
//!
//! ```sh
//! cargo run -p observatory_example                          # a window on macOS
//! cargo run -p observatory_example -- snapshot sky.png      # an AppKit PNG on macOS
//! cargo run -p observatory_example -- summary               # the screen's semantics, anywhere
//! ```

use std::process::ExitCode;

use leptoskit_testing::WidgetTester;
use observatory_example::Observatory;
use observatory_example::VIEWPORT;

fn main() -> ExitCode {
	let mut args = std::env::args().skip(1);
	let result = match args.next().as_deref() {
		None => show(),
		Some("summary") => print_summary(),
		Some("snapshot") => {
			match args.next() {
				Some(path) => snapshot(path.as_ref()),
				None => Err("snapshot needs a path for the PNG".to_owned()),
			}
		}
		Some(other) => {
			Err(format!(
				"unknown command `{other}`; try summary or snapshot <path>"
			))
		}
	};

	match result {
		Ok(()) => ExitCode::SUCCESS,
		Err(message) => {
			eprintln!("observatory: {message}");
			ExitCode::FAILURE
		}
	}
}

/// Renders the screen on the headless backend and prints the accessibility
/// tree a screen reader would walk.
fn print_summary() -> Result<(), String> {
	let observatory = Observatory::new();
	let tester = WidgetTester::new(move || observatory.render(), VIEWPORT.0, VIEWPORT.1)
		.map_err(|error| error.to_string())?;
	print!("{}", tester.semantics());
	Ok(())
}

#[cfg(target_os = "macos")]
mod window {
	use std::cell::RefCell;
	use std::path::Path;
	use std::rc::Rc;

	use leptoskit::appkit::AppKitBackend;
	use leptoskit::appkit::MainThreadMarker;
	use leptoskit::appkit::run_application;
	use leptoskit::native::NativeSurface;
	use observatory_example::Observatory;
	use observatory_example::VIEWPORT;

	/// The pixels per point of a snapshot: retina quality.
	const SNAPSHOT_SCALE: f64 = 2.0;

	/// The screen mounted in AppKit views.
	fn mount(mtm: MainThreadMarker, observatory: &Observatory) -> NativeSurface<AppKitBackend> {
		let mut surface = NativeSurface::new(AppKitBackend::new(mtm));
		surface.set_dark_mode(true);
		surface.set_viewport(VIEWPORT.0, VIEWPORT.1);
		surface.set_root(&observatory.render());
		surface
	}

	fn main_thread() -> Result<MainThreadMarker, String> {
		MainThreadMarker::new().ok_or_else(|| "AppKit must run on the main thread".to_owned())
	}

	/// Renders the screen with real AppKit views, offscreen, to a PNG.
	pub fn snapshot(path: &Path) -> Result<(), String> {
		let surface = mount(main_thread()?, &Observatory::new());
		let root = surface.root().ok_or("the screen committed no root")?;
		let png = surface
			.backend()
			.snapshot_png(root, SNAPSHOT_SCALE)
			.map_err(|error| error.to_string())?;
		std::fs::write(path, png)
			.map_err(|error| format!("could not write {}: {error}", path.display()))
	}

	/// Mounts the screen in AppKit views, re-renders it after every event,
	/// and runs the application until its window closes.
	pub fn open() -> Result<(), String> {
		let mtm = main_thread()?;
		let observatory = Observatory::new();
		let surface = Rc::new(RefCell::new(mount(mtm, &observatory)));

		// AppKit delivers events from its own actions, never while the
		// surface is borrowed, so the listener dispatches and re-renders in
		// place.
		let weak = Rc::downgrade(&surface);
		surface.borrow().backend().set_event_listener({
			let observatory = observatory.clone();
			move |node, event| {
				if let Some(surface) = weak.upgrade() {
					let mut surface = surface.borrow_mut();
					surface.dispatch_event(node, event);
					surface.set_root(&observatory.render());
				}
			}
		});

		let surface_ref = surface.borrow();
		let root = surface_ref.root().ok_or("the screen committed no root")?;
		let _window = surface_ref
			.backend()
			.open_window(root, "Observatory")
			.ok_or("the root view disappeared before the window opened")?;
		drop(surface_ref);
		run_application(mtm);
		Ok(())
	}
}

#[cfg(target_os = "macos")]
fn show() -> Result<(), String> {
	window::open()
}

#[cfg(target_os = "macos")]
fn snapshot(path: &std::path::Path) -> Result<(), String> {
	window::snapshot(path)
}

/// Windows open on macOS; elsewhere the summary stands in.
#[cfg(not(target_os = "macos"))]
fn show() -> Result<(), String> {
	eprintln!("observatory: windows open on macOS; printing the summary instead");
	print_summary()
}

#[cfg(not(target_os = "macos"))]
fn snapshot(_path: &std::path::Path) -> Result<(), String> {
	Err("snapshots render AppKit views, so they need macOS".to_owned())
}
