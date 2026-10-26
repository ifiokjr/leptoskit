//! The gallery's terminal binary: `gallery-tui <screen>` runs one shared
//! screen live in the cell grid, with keyboard focus and mouse clicks.

use std::process::ExitCode;

use gallery::SCREENS;
use gallery::tui;

const USAGE: &str = "usage: gallery-tui [screen]

The screen runs live in this terminal: tab moves focus, enter presses, and
ctrl+c exits. Try one of:

";

fn main() -> ExitCode {
	let slug = std::env::args().nth(1).unwrap_or_else(|| "counter".into());

	let Some(screen) = gallery::find_screen(&slug) else {
		eprint!("{USAGE}");
		for screen in &SCREENS {
			eprintln!("  {}", screen.slug);
		}
		return ExitCode::from(2);
	};

	let assets = match gallery::native::default_assets() {
		Ok(assets) => assets,
		Err(error) => {
			eprintln!("{error}");
			return ExitCode::FAILURE;
		}
	};

	if let Err(error) = tui::run(screen, assets) {
		eprintln!("terminal error: {error}");
		return ExitCode::FAILURE;
	}

	ExitCode::SUCCESS
}
