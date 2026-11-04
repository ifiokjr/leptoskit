//! The self-hosted deployment service and `LeptosKit` dashboard. The `server`
//! feature enables its durable API/worker; the older design prototype remains
//! available as an independent example.

#[cfg(feature = "server")]
pub mod control;

#[cfg(feature = "server")]
pub mod hosted;

pub mod hosted_protocol;
pub mod model;
pub mod store;
pub mod view;

#[cfg(all(target_arch = "wasm32", feature = "web"))]
#[leptos::component]
pub fn App() -> impl leptos::prelude::IntoView {
	move || leptoskit::web::render(store::read(view::screen))
}

#[cfg(all(target_arch = "wasm32", feature = "web"))]
#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn start() {
	leptos::mount::mount_to_body(App);
}
