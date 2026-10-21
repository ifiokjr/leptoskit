//! The gallery's desktop binary.
//!
//! - `gallery list` prints every screen.
//! - `gallery snapshot <out_dir>` renders every screen to
//!   `<out_dir>/<slug>.png` with the platform's native widgets.
//! - `gallery window <screen>` shows one screen in a native window, live:
//!   controls dispatch through the renderer and the screen re-renders.
//!
//! macOS renders with AppKit, Linux with GTK 4, and Windows with Win32. The
//! mobile hosts are `gallery-ios` and the Android `GalleryActivity`.

use std::process::ExitCode;

use gallery::SCREENS;

#[cfg(target_os = "macos")]
#[path = "platform/appkit.rs"]
mod platform;
#[cfg(target_os = "linux")]
#[path = "platform/gtk.rs"]
mod platform;
#[cfg(target_os = "windows")]
#[path = "platform/windows.rs"]
mod platform;

const USAGE: &str = "usage: gallery <command>

commands:
  list                  print the screens
  snapshot <out_dir>    render every screen to <out_dir>/<screen>.png
  window <screen>       show one screen in a native window";

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
		["snapshot", out_dir] => report(platform::snapshot(std::path::Path::new(out_dir))),
		["window", slug] => report(platform::window(slug)),
		_ => {
			eprintln!("{USAGE}");
			ExitCode::from(2)
		}
	}
}

/// The gallery's images, written to their default directory.
#[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
fn assets() -> Result<gallery::Assets, String> {
	let dir = gallery::Assets::default_dir();
	gallery::Assets::generate(&dir).map_err(|error| {
		format!(
			"could not write the gallery images to {}: {error}",
			dir.display()
		)
	})
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

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
mod platform {
	use std::path::Path;

	fn unsupported() -> Result<(), String> {
		Err(format!(
			"this binary has no desktop host on {}; run `gallery-ios` on iOS or the Android \
			 `GalleryActivity` (see examples/gallery/readme.md)",
			std::env::consts::OS
		))
	}

	pub fn snapshot(_out_dir: &Path) -> Result<(), String> {
		unsupported()
	}

	pub fn window(_slug: &str) -> Result<(), String> {
		unsupported()
	}
}
