//! Opens the postcard in a macOS window, or prints what it would show.
//!
//! ```sh
//! cargo run -p postcard_example                        # a window on macOS, in your language
//! cargo run -p postcard_example -- snapshot card.png   # an AppKit PNG on macOS
//! cargo run -p postcard_example -- summary ar          # the Arabic screen's semantics, anywhere
//! ```
//!
//! The window starts in the language `LANG` names, when the app has it.

use std::process::ExitCode;

use leptoskit_l10n::Locale;
use leptoskit_testing::WidgetTester;
use postcard_example::PostcardApp;
use postcard_example::VIEWPORT;

fn main() -> ExitCode {
	let mut args = std::env::args().skip(1);
	let result = match args.next().as_deref() {
		None => show(&preferred(None)),
		Some("summary") => print_summary(&preferred(args.next())),
		Some("snapshot") => {
			match args.next() {
				Some(path) => snapshot(path.as_ref(), &preferred(args.next())),
				None => Err("snapshot needs a path for the PNG".to_owned()),
			}
		}
		Some(other) => {
			Err(format!(
				"unknown command `{other}`; try summary [tag] or snapshot <path> [tag]"
			))
		}
	};

	match result {
		Ok(()) => ExitCode::SUCCESS,
		Err(message) => {
			eprintln!("postcard: {message}");
			ExitCode::FAILURE
		}
	}
}

/// The preferred language: the tag given, or the system's `LANG`, such as
/// `pt_BR.UTF-8`.
fn preferred(tag: Option<String>) -> Vec<Locale> {
	let tag = tag
		.or_else(|| std::env::var("LANG").ok())
		.unwrap_or_default();
	let tag = tag.split('.').next().unwrap_or_default().replace('_', "-");
	Locale::new(&tag).into_iter().collect()
}

/// Renders the screen on the headless backend and prints the accessibility
/// tree a screen reader would walk.
fn print_summary(preferred: &[Locale]) -> Result<(), String> {
	let app = PostcardApp::new(preferred);
	let tester = WidgetTester::new(move || app.render(), VIEWPORT.0, VIEWPORT.1)
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
	use leptoskit_l10n::Locale;
	use postcard_example::PostcardApp;
	use postcard_example::VIEWPORT;

	/// The pixels per point of a snapshot: retina quality.
	const SNAPSHOT_SCALE: f64 = 2.0;

	fn mount(mtm: MainThreadMarker, app: &PostcardApp) -> NativeSurface<AppKitBackend> {
		let mut surface = NativeSurface::new(AppKitBackend::new(mtm));
		surface.set_dark_mode(false);
		surface.set_viewport(VIEWPORT.0, VIEWPORT.1);
		surface.set_root(&app.render());
		surface
	}

	fn main_thread() -> Result<MainThreadMarker, String> {
		MainThreadMarker::new().ok_or_else(|| "AppKit must run on the main thread".to_owned())
	}

	/// Renders the screen with real AppKit views, offscreen, to a PNG.
	pub fn snapshot(path: &Path, preferred: &[Locale]) -> Result<(), String> {
		let surface = mount(main_thread()?, &PostcardApp::new(preferred));
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
	pub fn open(preferred: &[Locale]) -> Result<(), String> {
		let mtm = main_thread()?;
		let app = PostcardApp::new(preferred);
		let surface = Rc::new(RefCell::new(mount(mtm, &app)));

		// AppKit delivers events from its own actions, never while the
		// surface is borrowed, so the listener dispatches and re-renders in
		// place.
		let weak = Rc::downgrade(&surface);
		surface.borrow().backend().set_event_listener({
			let app = app.clone();
			move |node, event| {
				if let Some(surface) = weak.upgrade() {
					let mut surface = surface.borrow_mut();
					surface.dispatch_event(node, event);
					surface.set_root(&app.render());
				}
			}
		});

		let surface_ref = surface.borrow();
		let root = surface_ref.root().ok_or("the screen committed no root")?;
		let _window = surface_ref
			.backend()
			.open_window(root, "Postcard")
			.ok_or("the root view disappeared before the window opened")?;
		drop(surface_ref);
		run_application(mtm);
		Ok(())
	}
}

#[cfg(target_os = "macos")]
fn show(preferred: &[Locale]) -> Result<(), String> {
	window::open(preferred)
}

#[cfg(target_os = "macos")]
fn snapshot(path: &std::path::Path, preferred: &[Locale]) -> Result<(), String> {
	window::snapshot(path, preferred)
}

/// Windows open on macOS; elsewhere the summary stands in.
#[cfg(not(target_os = "macos"))]
fn show(preferred: &[Locale]) -> Result<(), String> {
	eprintln!("postcard: windows open on macOS; printing the summary instead");
	print_summary(preferred)
}

#[cfg(not(target_os = "macos"))]
fn snapshot(_path: &std::path::Path, _preferred: &[Locale]) -> Result<(), String> {
	Err("snapshots render AppKit views, so they need macOS".to_owned())
}
