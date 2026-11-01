#![cfg(all(target_arch = "wasm32", target_os = "unknown", feature = "web"))]

use leptoskit_bridge::Bytes;
use leptoskit_bridge::Error;
use leptoskit_bridge::Plugin;
use leptoskit_plugin_crypto::Crypto;
use leptoskit_plugin_crypto::MAX_RANDOM_BYTES;
use leptoskit_plugin_crypto::RANDOM_BYTES;
use leptoskit_plugin_crypto::UUID_V4;
use leptoskit_plugin_crypto::UuidV4;
use leptoskit_plugin_crypto::fill_random;
use leptoskit_plugin_crypto::random_bytes;
use leptoskit_plugin_crypto::uuid_v4;
use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

#[wasm_bindgen_test]
fn browser_webcrypto_random_bytes_and_uuid_work() {
	let browser_crypto = web_sys::window().unwrap().crypto().unwrap();
	let mut browser_buffer = [0; 32];
	browser_crypto
		.get_random_values_with_u8_array(&mut browser_buffer)
		.unwrap();

	for length in [0, 1, 32, MAX_RANDOM_BYTES] {
		assert_eq!(random_bytes(length).unwrap().len(), length);
	}

	let mut reusable = [0; 32];
	fill_random(&mut reusable).unwrap();
	let uuid = uuid_v4().unwrap();

	assert_eq!(uuid.as_bytes()[6] >> 4, 4);
	assert_eq!(uuid.as_bytes()[8] >> 6, 2);
	assert_eq!(uuid.to_string().len(), 36);
	UuidV4::decode(&Crypto.invoke(UUID_V4, Bytes::new()).unwrap()).unwrap();
	assert_eq!(
		Crypto
			.invoke(RANDOM_BYTES, Bytes::copy_from_slice(&4_u32.to_le_bytes()))
			.unwrap()
			.len(),
		4
	);
	assert_eq!(random_bytes(MAX_RANDOM_BYTES + 1), Err(Error::TooLarge));
}

#[wasm_bindgen_test]
fn failed_webcrypto_does_not_fall_back_to_predictable_randomness() {
	let crypto = web_sys::window().unwrap().crypto().unwrap();
	let key = JsValue::from_str("getRandomValues");
	let original = js_sys::Reflect::get(&crypto, &key).unwrap();
	let descriptor = js_sys::Object::get_own_property_descriptor(crypto.as_ref(), &key);
	let throw = js_sys::Function::new_no_args("throw new Error('entropy unavailable for test');");
	assert!(js_sys::Reflect::set(&crypto, &key, throw.as_ref()).unwrap());
	let random_result = random_bytes(32);
	let uuid_result = uuid_v4();

	// Restore the real browser function before making assertions.
	if descriptor.is_undefined() {
		assert!(js_sys::Reflect::delete_property(crypto.as_ref(), &key).unwrap());
	} else {
		assert!(
			js_sys::Reflect::define_property(crypto.as_ref(), &key, descriptor.unchecked_ref())
				.unwrap()
		);
	}

	assert_eq!(random_result, Err(Error::Native));
	assert_eq!(uuid_result, Err(Error::Native));
	assert!(original.dyn_ref::<js_sys::Function>().is_some());
	assert!(random_bytes(32).is_ok());
}
