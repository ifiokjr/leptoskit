//! The leptoskit `DevTools` interface.
//!
//! A leptoskit app, written with leptoskit's own components, that shows what
//! every app running under `leptoskit dev` reports to the `DevTools` hub: the
//! element tree on each platform, a node's source, frame, and styles, the
//! route, queries, forms, named state, calls, and commits. The hub serves it
//! at `/`, and the browser `DevTools` panel frames it with `?embed=panel`
//! (see [`hub`]).
//!
//! [`model`] holds the rules, [`view`] draws them, [`store`] runs the
//! controls' actions, and [`hub`] talks to the hub.
//!
//! [`panel`] is the browser `DevTools` panel: the extension build of this app,
//! a small shell that frames the hub's interface rather than bundling it.

pub mod hub;
pub mod model;
pub mod panel;
pub mod store;
pub mod view;

/// The interface, drawn again whenever the model changes.
#[cfg(all(target_arch = "wasm32", not(feature = "extension")))]
#[leptos::component]
pub fn App() -> impl leptos::prelude::IntoView {
	move || leptoskit::web::render(view::page())
}

/// Mounts the interface when the page loads its Wasm module. The extension's
/// pages start the panel instead, through `entry!`.
#[cfg(all(target_arch = "wasm32", not(feature = "extension")))]
#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn start() {
	leptos::mount::mount_to_body(App);
	hub::start();
}

// The panel's two pages; see `panel`.
#[cfg(feature = "extension")]
leptoskit_extension::entry!(main: panel::main);
