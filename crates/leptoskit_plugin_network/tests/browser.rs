#![cfg(all(target_arch = "wasm32", target_os = "unknown", not(feature = "guest")))]

use leptoskit_plugin_network::Connection;
use leptoskit_plugin_network::Snapshot;
use leptoskit_plugin_network::web;
use wasm_bindgen_test::wasm_bindgen_test;

wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

#[wasm_bindgen_test]
async fn stale_reading_returns_current_state_without_waiting_for_an_event() {
	let stale = Snapshot::connection(Connection::Unknown, None);
	let current = web::wait_for_change(stale).await.unwrap();
	assert!(current.connected.is_some());
	assert_eq!(current.internet_validated, None);
	assert_eq!(Snapshot::decode(&current.encode().unwrap()), Ok(current));
}
