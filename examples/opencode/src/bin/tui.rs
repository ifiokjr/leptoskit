//! `opencode`: the interface live in this terminal.
//!
//! Type a prompt and press enter to play the scripted session. ctrl+p opens
//! the command palette, shift+tab switches agents, and ctrl+c quits.

#[cfg(not(target_arch = "wasm32"))]
fn main() -> std::process::ExitCode {
	use opencode_example::terminal;

	let mut app = terminal::TerminalApp::new(terminal::working_directory());

	match leptoskit::tui::terminal::run_app(terminal::options(), &mut app) {
		Ok(_) => std::process::ExitCode::SUCCESS,
		Err(error) => {
			eprintln!("{error}");
			std::process::ExitCode::FAILURE
		}
	}
}

#[cfg(target_arch = "wasm32")]
fn main() {
	eprintln!("Run `trunk serve` in examples/opencode for the browser version.");
}
