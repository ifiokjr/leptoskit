#![cfg(all(
	target_arch = "wasm32",
	target_os = "unknown",
	feature = "browser-integration"
))]

use leptoskit_plugin_websocket::Client;
use leptoskit_plugin_websocket::Config;
use leptoskit_plugin_websocket::Error;
use leptoskit_plugin_websocket::Message;
use wasm_bindgen_test::wasm_bindgen_test;

wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

fn origin() -> Result<&'static str, &'static str> {
	option_env!("LEPTOSKIT_WEBSOCKET_TEST_URL")
		.ok_or("start browser_test_server and set LEPTOSKIT_WEBSOCKET_TEST_URL when building tests")
}

#[wasm_bindgen_test]
async fn browser_binary_echo_and_close_use_a_real_local_socket() {
	let origin = origin().unwrap();
	let client = Client::new(Config::new([origin]).unwrap()).unwrap();
	let mut socket = client
		.connect(&format!("{origin}/echo"), &[])
		.await
		.unwrap();
	let data = Message::Binary(vec![0, 255, 128, 42]);
	socket.send(data.clone()).await.unwrap();
	assert_eq!(socket.receive().await.unwrap(), Some(data));
	socket.close(1000, "done").await.unwrap();
	assert!(socket.receive().await.unwrap().is_none());
}

#[wasm_bindgen_test]
async fn browser_rejects_an_oversized_incoming_message() {
	let origin = origin().unwrap();
	let mut config = Config::new([origin]).unwrap();
	config.max_message_bytes = 8;
	let mut socket = Client::new(config)
		.unwrap()
		.connect(&format!("{origin}/large"), &[])
		.await
		.unwrap();
	assert!(matches!(socket.receive().await, Err(Error::TooLarge)));
}

#[wasm_bindgen_test]
async fn browser_empty_message_flood_is_bounded_by_item_count() {
	let origin = origin().unwrap();
	let mut config = Config::new([origin]).unwrap();
	config.max_pending_messages = 2;
	let mut socket = Client::new(config)
		.unwrap()
		.connect(&format!("{origin}/flood"), &[])
		.await
		.unwrap();
	gloo_timers::future::TimeoutFuture::new(100).await;
	assert!(matches!(socket.receive().await, Err(Error::Backpressure)));
}
