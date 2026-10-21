#![cfg(all(target_arch = "wasm32", target_os = "unknown"))]

use leptoskit_plugin_http::Client;
use leptoskit_plugin_http::Config;
use leptoskit_plugin_http::Error;
use leptoskit_plugin_http::Request;
use wasm_bindgen_test::wasm_bindgen_test;

wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

#[wasm_bindgen_test]
async fn fetches_local_test_server_and_enforces_streamed_body_limit() {
	let origin = web_sys::window().unwrap().location().origin().unwrap();
	let response = Client::new(Config::new([&origin]).unwrap())
		.unwrap()
		.execute(Request::get(format!("{origin}/")))
		.await
		.unwrap();
	assert_eq!(response.status, 200);
	assert!(!response.body.is_empty());
	let mut config = Config::new([&origin]).unwrap();
	config.max_body_bytes = 1;
	assert!(matches!(
		Client::new(config)
			.unwrap()
			.execute(Request::get(format!("{origin}/")))
			.await,
		Err(Error::TooLarge)
	));
}
