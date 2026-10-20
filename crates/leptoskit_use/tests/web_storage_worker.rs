#![cfg(all(
	feature = "local-storage",
	target_arch = "wasm32",
	target_os = "unknown"
))]

use leptoskit_use::LocalStorage;
use leptoskit_use::StorageError;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_dedicated_worker);

#[wasm_bindgen_test]
fn hosts_without_a_window_get_an_explicit_capability_error() {
	assert!(matches!(
		LocalStorage::<String>::new("worker"),
		Err(StorageError::Unavailable(_))
	));
}
