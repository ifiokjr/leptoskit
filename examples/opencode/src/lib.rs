//! An opencode-style coding agent interface, written once as file routes over
//! shared components and run in the browser and in an interactive terminal.
//!
//! The screens match opencode 2's terminal interface cell for cell: the
//! two-tone logo, the prompt with its agent bar, a session with markdown, a
//! diff, and shell output, the context sidebar, and the command palette over a
//! dimming scrim. Every model reply is scripted, so the example runs offline.
//!
//! `src/routes/` holds the screens and `leptoskit routes` writes
//! `src/routes.rs` from it. [`Store`] holds the state every route reads. The
//! leptoskit CLI builds both hosts: `leptoskit dev` serves the browser
//! version, and `leptoskit dev --platform tui` runs the terminal one.
//!
//! ```
//! use opencode_example::routes::Navigation;
//! use opencode_example::routes::Route;
//!
//! let navigation = Navigation::new(Route::Home);
//! let screen = opencode_example::screen(&navigation);
//! assert!(!screen.children.is_empty());
//! ```

use leptoskit_native::Element;

mod demo;
mod highlight;
mod logo;
mod markdown;
pub mod routes;
mod state;
pub mod theme;
mod ui;

pub use state::Action;
pub use state::Agent;
pub use state::Change;
pub use state::DiffLine;
pub use state::Message;
pub use state::Model;
pub use state::Opencode;
pub use state::Palette;
pub use state::Part;
pub use state::Session;
pub use state::Store;

/// A no-break space: a blank that never wraps or collapses, so indentation
/// and the logo's spacing survive in the terminal and the browser alike.
pub const NBSP: char = '\u{a0}';

/// The version the interface shows in its corner.
pub const VERSION: &str = "2.0.15";

/// The interface for any host: the current route inside its layouts.
#[must_use]
pub fn screen(navigation: &routes::Navigation) -> Element {
	navigation.render()
}

/// Mounts the interface when the browser loads the wasm module, following
/// the address bar and the window's size.
#[cfg(all(target_arch = "wasm32", feature = "web"))]
#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn start() {
	use leptos::prelude::*;

	/// The window's size in the terminal's 8×16 cells.
	fn cells(window: &web_sys::Window) -> (u16, u16) {
		let size = |value: Result<wasm_bindgen::JsValue, _>, cell: f64| {
			(value.ok().and_then(|value| value.as_f64()).unwrap_or(0.0) / cell) as u16
		};
		(
			size(window.inner_width(), 8.0),
			size(window.inner_height(), 16.0),
		)
	}

	let store = Store::current();
	let (cols, rows) = cells(&window());
	store.resize(cols, rows);
	let navigation = routes::Navigation::from_browser()
		.unwrap_or_else(|_| routes::Navigation::new(routes::Route::Home));

	mount_to_body(move || {
		let _resize = window_event_listener(leptos::ev::resize, move |_| {
			let (cols, rows) = cells(&window());
			store.resize(cols, rows);
		});
		move || leptoskit::web::render(screen(&navigation))
	});
}

#[cfg(all(target_arch = "wasm32", feature = "native"))]
mod guest;
