//! <!-- {=deploy_consoleSummary|trim|linePrefix:"//! ":true} -->
//!
//! One optional service hosts the dashboard, durable operation queue, scoped
//! agent API and analytics history. It uses SQLite and reuses the existing
//! `OpenTofu`/Kamal engine through fixed CLI operations. No Redis or separate
//! monitoring database is required.
//!
//! <!-- {/deploy_consoleSummary} -->

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
