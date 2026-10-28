#![cfg(all(feature = "web", target_arch = "wasm32", target_os = "unknown"))]

use leptoskit_use::web::use_media_query;
use leptoskit_use::web::use_preferred_dark;
use reactive_graph::owner::Owner;
use reactive_graph::prelude::Get;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_dedicated_worker);

#[wasm_bindgen_test]
fn without_a_window_media_queries_stay_false() {
	let owner = Owner::new();
	owner.with(|| {
		assert!(!use_media_query("(min-width: 1px)").get());
		assert!(!use_preferred_dark().get());
	});
	owner.cleanup();
}
