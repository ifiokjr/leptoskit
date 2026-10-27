#![allow(unsafe_code)] // Exported C names form the native runtime's entry points.

//! Field, a notes app whose screens are files.
//!
//! `src/routes/` holds one file per screen, and the router plugin writes
//! `src/routes.rs` from it: a module per file and the `Route` enum. A
//! [`Navigation`](routes::Navigation) router renders the current route inside
//! its layouts:
//!
//! ```
//! use file_routes_example::routes::Navigation;
//! use file_routes_example::routes::Route;
//!
//! let navigation = Navigation::new(Route::Notes);
//! let screen = navigation.render();
//! assert!(!screen.children.is_empty());
//! ```

pub mod routes;

/// A note's identifier in the URL.
pub type NoteId = u64;

/// The browser application: the address bar picks the first route, and links
/// and the back button move through history.
#[cfg(all(feature = "web", target_arch = "wasm32"))]
#[leptos::component]
pub fn App() -> impl leptos::prelude::IntoView {
	let navigation = routes::Navigation::from_browser()
		.unwrap_or_else(|_| routes::Navigation::new(routes::Route::Today));
	move || leptoskit::web::render(navigation.render())
}

/// Mounts the browser application when the page loads its Wasm module.
#[cfg(all(target_arch = "wasm32", feature = "web"))]
#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn start() {
	leptos::mount::mount_to_body(App);
}

/// Mounts the same application in the native Wasm runtime.
#[cfg(all(target_arch = "wasm32", feature = "native"))]
#[unsafe(no_mangle)]
pub extern "C" fn leptoskit_main() {
	let navigation = routes::Navigation::new(routes::Route::Today);
	leptoskit::guest::mount(move || navigation.render());
}

/// Forwards platform-neutral events to the shared tree's handlers.
#[cfg(all(target_arch = "wasm32", feature = "native"))]
#[unsafe(no_mangle)]
pub extern "C" fn leptoskit_event(node: i64, kind: i32, ptr: i32, len: i32) -> i32 {
	leptoskit::guest::dispatch(node, kind, ptr, len)
}

/// Allocates the native runtime's event payload buffer.
#[cfg(all(target_arch = "wasm32", feature = "native"))]
#[unsafe(no_mangle)]
pub extern "C" fn leptoskit_alloc(len: i32) -> i32 {
	leptoskit::guest::alloc(len)
}

/// Reports the native UI bridge ABI implemented by this app.
#[cfg(all(target_arch = "wasm32", feature = "native"))]
#[unsafe(no_mangle)]
pub extern "C" fn leptoskit_abi_version() -> i32 {
	leptoskit::guest::ABI_VERSION as i32
}
