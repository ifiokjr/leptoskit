//! A leptos islands app built by leptoskit.
//!
//! Every page renders on the server. The wasm client carries only the
//! `#[island]` components, and hydrates only the parts of the page they
//! render: the prose around them never ships a byte of wasm.

pub mod app;

/// The client entry point: `HydrationScripts` calls it once the wasm loads.
#[cfg(feature = "hydrate")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn hydrate() {
	console_error_panic_hook::set_once();
	leptos::mount::hydrate_islands();
}
