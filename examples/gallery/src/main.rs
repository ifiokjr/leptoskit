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
#[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
use gallery::desktop;

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
		["snapshot", out_dir] => {
			report(
				desktop::snapshot(std::path::Path::new(out_dir)).map(|paths| {
					for path in paths {
						println!("{}", path.display());
					}
				}),
			)
		}
		["window", slug] => report(desktop::window(slug)),
		_ => {
			eprintln!("{USAGE}");
			ExitCode::from(2)
		}
	}
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
mod desktop {
	use std::path::Path;
	use std::path::PathBuf;

	fn unsupported<T>() -> Result<T, String> {
		Err(format!(
			"this binary has no desktop host on {}; run `gallery-ios` on iOS or the Android \
			 `GalleryActivity` (see examples/gallery/readme.md)",
			std::env::consts::OS
		))
	}

	pub fn snapshot(_out_dir: &Path) -> Result<Vec<PathBuf>, String> {
		unsupported()
	}

	pub fn window(_slug: &str) -> Result<(), String> {
		unsupported()
	}
}
