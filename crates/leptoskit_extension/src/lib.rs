#![cfg_attr(docsrs, feature(doc_cfg))]

//! Browser extensions for leptoskit apps.
//!
//! An extension is the app's own routes in a few new places: the toolbar
//! popup, the side panel, the options page, the new tab page, and frames on
//! other sites' pages. `leptoskit build --platform extension` builds one wasm
//! client and packages it for Chromium, Firefox, and Safari. The app names
//! its entry points once:
//!
//! ```ignore
//! leptoskit::extension::entry!(main: extension, background: background);
//!
//! /// Every surface opens at its configured route.
//! fn extension(surface: Surface) -> impl Fn() -> Element + 'static {
//! 	let navigation = Navigation::new(Route::parse(surface.route()).unwrap_or(Route::Home));
//! 	move || navigation.render()
//! }
//!
//! /// The worker without a screen: register what it answers.
//! fn background(worker: &Background) {
//! 	worker.on_message(|message: Message, _sender| async move { reply(message).await });
//! }
//! ```
//!
//! Every API in this crate works the same in each browser family, returns
//! [`Error::Unavailable`] where a browser lacks it, and stops listening when
//! its handle is dropped. Outside a browser every call is unavailable, so the
//! app's code compiles and its tests run on any host.

mod js;

pub mod action;
pub mod alarms;
pub mod background;
pub mod commands;
pub mod context_menus;
pub mod notifications;
pub mod page;
pub mod permissions;
pub mod runtime;
pub mod scripting;
pub mod side_panel;
pub mod storage;
mod surface;
pub mod tabs;

pub use background::Background;
pub use js::Listener;
pub use surface::PageName;
pub use surface::Surface;

/// Why an extension API call failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
	/// This browser has no such API, or the extension lacks the permission
	/// that exposes it. The value names the API.
	#[error("`{0}` isn't available in this browser")]
	Unavailable(String),
	/// The browser refused the call, with its reason.
	#[error("{0}")]
	Browser(String),
	/// A value couldn't cross between Rust and JavaScript.
	#[error("couldn't convert a value: {0}")]
	Json(String),
}

/// A result from the extension API.
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Names the app's extension entry points: `main`, which renders every
/// surface with a screen, and optionally `background`, which registers the
/// worker's handlers.
///
/// `main` takes the [`Surface`] being opened and returns the function that
/// renders it, which runs again whenever a signal it read changes. Like any
/// leptos view, that function must be `Send + Sync`.
/// `background` takes the [`Background`] to register handlers on.
///
/// The macro exports the functions the generated pages and worker call, so
/// use it once, in the app's library crate, and keep the app's own
/// `#[wasm_bindgen(start)]` out of extension builds:
/// `#[cfg(all(feature = "web", not(feature = "extension")))]`.
#[macro_export]
macro_rules! entry {
	(main: $main:path $(,)?) => {
		$crate::__entry_main!($main);
	};
	(main: $main:path,background: $background:path $(,)?) => {
		$crate::__entry_main!($main);

		/// Called once by the generated worker when the wasm is ready.
		#[cfg(target_arch = "wasm32")]
		#[$crate::__private::export(wasm_bindgen = $crate::__private::wasm_bindgen)]
		pub fn leptoskit_extension_background() {
			$crate::__private::start_background($background);
		}

		/// Called by the generated worker for every event it queued or
		/// received, with the event's details; resolves to the reply.
		#[cfg(target_arch = "wasm32")]
		#[$crate::__private::export(wasm_bindgen = $crate::__private::wasm_bindgen)]
		pub fn leptoskit_extension_event(
			kind: ::std::string::String,
			detail: $crate::__private::wasm_bindgen::JsValue,
		) -> $crate::__private::js_sys::Promise {
			$crate::__private::dispatch(&kind, detail)
		}
	};
}

#[doc(hidden)]
#[macro_export]
macro_rules! __entry_main {
	($main:path) => {
		/// Called by each generated page with its surface and route.
		#[cfg(target_arch = "wasm32")]
		#[$crate::__private::export(wasm_bindgen = $crate::__private::wasm_bindgen)]
		pub fn leptoskit_extension_start(
			surface: ::std::string::String,
			route: ::std::string::String,
		) {
			$crate::__private::start(&surface, &route, $main);
		}
	};
}

#[doc(hidden)]
pub mod __private {
	#[cfg(target_arch = "wasm32")]
	pub use js_sys;
	#[cfg(target_arch = "wasm32")]
	pub use wasm_bindgen;
	/// `#[wasm_bindgen]`, by a name short enough that the attribute fits on
	/// one line, which keeps rustfmt from re-indenting it on every run.
	#[cfg(target_arch = "wasm32")]
	pub use wasm_bindgen::prelude::wasm_bindgen as export;

	#[cfg(target_arch = "wasm32")]
	pub use crate::background::dispatch;
	#[cfg(target_arch = "wasm32")]
	pub use crate::background::start as start_background;
	#[cfg(target_arch = "wasm32")]
	pub use crate::surface::start;
}
