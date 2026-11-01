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

pub mod hub;
pub mod model;
pub mod store;
pub mod view;

/// The interface, drawn again whenever the model changes.
#[cfg(target_arch = "wasm32")]
#[leptos::component]
pub fn App() -> impl leptos::prelude::IntoView {
	move || leptoskit::web::render(view::page())
}

/// Mounts the interface when the page loads its Wasm module.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn start() {
	leptos::mount::mount_to_body(App);
	hub::start();
}
