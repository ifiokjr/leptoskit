//! The gallery binary.
//!
//! - `gallery list` prints every screen.
//! - `gallery snapshot <out_dir>` renders every screen to
//!   `<out_dir>/<slug>.png` at 2x with the AppKit backend.
//! - `gallery window <screen>` shows one screen in a window, live: buttons
//!   dispatch through the renderer and the screen re-renders.
//!
//! `snapshot` and `window` need AppKit, so they only work on macOS; elsewhere
//! they explain why and exit non-zero.

use std::process::ExitCode;

use gallery::SCREENS;

const USAGE: &str = "usage: gallery <command>

commands:
  list                  print the screens
  snapshot <out_dir>    render every screen to <out_dir>/<screen>.png (macOS)
  window <screen>       show one screen in a window (macOS)";

fn main() -> ExitCode {
	let args: Vec<String> = std::env::args().skip(1).collect();
	match args
		.iter()
		.map(String::as_str)
		.collect::<Vec<_>>()
		.as_slice()
	{
		["list"] => {
			for screen in &SCREENS {
				let (width, height) = screen.viewport;
				println!("{:<10} {} ({width}×{height})", screen.slug, screen.title);
			}
			ExitCode::SUCCESS
		}
		["snapshot", out_dir] => platform::snapshot(std::path::Path::new(out_dir)),
		["window", slug] => platform::window(slug),
		_ => {
			eprintln!("{USAGE}");
			ExitCode::from(2)
		}
	}
}

#[cfg(target_os = "macos")]
mod platform {
	use std::cell::RefCell;
	use std::path::Path;
	use std::process::ExitCode;
	use std::rc::Rc;

	use gallery::Assets;
	use gallery::SCREENS;
	use gallery::Screen;
	use gallery::ScreenState;
	use gallery::find_screen;
	use leptoskit_appkit::AppKitBackend;
	use leptoskit_appkit::MainThreadMarker;
	use leptoskit_appkit::run_application;
	use leptoskit_native::NativeSurface;

	/// The pixels per point of every snapshot: retina quality.
	const SNAPSHOT_SCALE: f64 = 2.0;

	fn main_thread() -> Result<MainThreadMarker, String> {
		MainThreadMarker::new().ok_or_else(|| "AppKit must run on the main thread".to_owned())
	}

	fn assets() -> Result<Assets, String> {
		let dir = Assets::default_dir();
		Assets::generate(&dir).map_err(|error| {
			format!(
				"could not write the gallery images to {}: {error}",
				dir.display()
			)
		})
	}

	/// A surface showing `screen`, laid out at its viewport.
	fn mount(
		mtm: MainThreadMarker,
		screen: &Screen,
		state: &ScreenState,
	) -> NativeSurface<AppKitBackend> {
		let mut surface = NativeSurface::new(AppKitBackend::new(mtm));
		// The gallery's fixed palette is light on every host.
		surface.set_dark_mode(false);
		let (width, height) = screen.viewport;
		surface.set_viewport(width, height);
		surface.set_root(&screen.render(state));
		surface
	}

	fn run_snapshot(out_dir: &Path) -> Result<(), String> {
		let mtm = main_thread()?;
		let assets = assets()?;
		std::fs::create_dir_all(out_dir)
			.map_err(|error| format!("could not create {}: {error}", out_dir.display()))?;
		for screen in &SCREENS {
			let state = ScreenState::new(assets.clone());
			let surface = mount(mtm, screen, &state);
			let root = surface.root().ok_or("the screen committed no root")?;
			let png = surface
				.backend()
				.snapshot_png(root, SNAPSHOT_SCALE)
				.map_err(|error| format!("could not snapshot `{}`: {error}", screen.slug))?;
			let path = out_dir.join(format!("{}.png", screen.slug));
			std::fs::write(&path, png)
				.map_err(|error| format!("could not write {}: {error}", path.display()))?;
			println!("{}", path.display());
		}
		Ok(())
	}

	pub fn snapshot(out_dir: &Path) -> ExitCode {
		report(run_snapshot(out_dir))
	}

	/// The live state of a window: the screen, its state, and its surface.
	struct Session {
		screen: &'static Screen,
		state: ScreenState,
		surface: NativeSurface<AppKitBackend>,
	}

	fn run_window(slug: &str) -> Result<(), String> {
		let mtm = main_thread()?;
		let screen = find_screen(slug).ok_or_else(|| {
			let known: Vec<&str> = SCREENS.iter().map(|screen| screen.slug).collect();
			format!("no screen named `{slug}`; try one of: {}", known.join(", "))
		})?;
		let state = ScreenState::new(assets()?);
		let surface = mount(mtm, screen, &state);
		let root = surface.root().ok_or("the screen committed no root")?;
		let session = Rc::new(RefCell::new(Session {
			screen,
			state,
			surface,
		}));

		// Events arrive from the AppKit run loop, never while the session is
		// borrowed, so the listener can dispatch and re-render in place.
		let weak = Rc::downgrade(&session);
		session
			.borrow()
			.surface
			.backend()
			.set_event_listener(move |node, event| {
				let Some(session) = weak.upgrade() else {
					return;
				};
				let mut session = session.borrow_mut();
				session.surface.dispatch_event(node, event);
				let tree = session.screen.render(&session.state);
				session.surface.set_root(&tree);
			});

		let _window = session
			.borrow()
			.surface
			.backend()
			.open_window(root, &format!("leptoskit · {}", screen.title))
			.ok_or("the root view disappeared before the window opened")?;
		run_application(mtm);
		Ok(())
	}

	pub fn window(slug: &str) -> ExitCode {
		report(run_window(slug))
	}

	fn report(result: Result<(), String>) -> ExitCode {
		match result {
			Ok(()) => ExitCode::SUCCESS,
			Err(error) => {
				eprintln!("gallery: {error}");
				ExitCode::FAILURE
			}
		}
	}
}

#[cfg(not(target_os = "macos"))]
mod platform {
	use std::path::Path;
	use std::process::ExitCode;

	fn unsupported(command: &str) -> ExitCode {
		eprintln!(
			"gallery: `{command}` renders with the AppKit backend, which only exists on macOS, \
			 and this is {}. The screens themselves are platform-independent: run `cargo test -p \
			 gallery` to lay every one of them out headlessly.",
			std::env::consts::OS
		);
		ExitCode::FAILURE
	}

	pub fn snapshot(_out_dir: &Path) -> ExitCode {
		unsupported("snapshot")
	}

	pub fn window(_slug: &str) -> ExitCode {
		unsupported("window")
	}
}
