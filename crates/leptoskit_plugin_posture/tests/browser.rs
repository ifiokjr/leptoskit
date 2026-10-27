#![cfg(all(target_arch = "wasm32", target_os = "unknown", not(feature = "guest")))]

use futures::FutureExt;
use leptoskit_plugin_posture::Axis;
use leptoskit_plugin_posture::Fold;
use leptoskit_plugin_posture::Posture;
use leptoskit_plugin_posture::web;
use wasm_bindgen::JsValue;
use wasm_bindgen_test::wasm_bindgen_test;

wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

/// Replaces a read-only browser property for the rest of the page's life.
fn define(target: &JsValue, key: &str, value: &JsValue) {
	let descriptor = js_sys::Object::new();
	js_sys::Reflect::set(&descriptor, &"value".into(), value).unwrap();
	js_sys::Reflect::set(&descriptor, &"configurable".into(), &true.into()).unwrap();
	js_sys::Reflect::set(&descriptor, &"writable".into(), &true.into()).unwrap();
	js_sys::Object::define_property(&target.clone().into(), &key.into(), &descriptor);
}

/// Two side-by-side segments with a gap between them, as on a dual-screen
/// device, and a posture object whose `type` the test can change.
fn fake_book_fold(posture_type: &str) -> (web_sys::Window, web_sys::EventTarget) {
	let window = web_sys::window().unwrap();
	let width = window.inner_width().unwrap().as_f64().unwrap();
	let height = window.inner_height().unwrap().as_f64().unwrap();
	let half = (width - 20.0) / 2.0;
	let segments = js_sys::Array::of2(
		&web_sys::DomRect::new_with_x_and_y_and_width_and_height(0.0, 0.0, half, height).unwrap(),
		&web_sys::DomRect::new_with_x_and_y_and_width_and_height(half + 20.0, 0.0, half, height)
			.unwrap(),
	);
	let viewport = js_sys::Object::new();
	js_sys::Reflect::set(&viewport, &"segments".into(), &segments).unwrap();
	define(window.as_ref(), "viewport", &viewport);
	let posture = web_sys::EventTarget::new().unwrap();
	js_sys::Reflect::set(&posture, &"type".into(), &posture_type.into()).unwrap();
	define(window.navigator().as_ref(), "devicePosture", &posture);
	(window, posture)
}

#[wasm_bindgen_test]
async fn segments_and_posture_become_hinges_and_wake_waiters() {
	let (_window, posture) = fake_book_fold("folded");
	let current = web::snapshot().unwrap();
	assert_eq!(current.fold, Fold::HalfOpen);
	assert_eq!(current.foldable, Some(true));
	assert!(current.is_book());
	let hinge = current.hinges[0];
	assert_eq!(hinge.axis, Axis::Vertical);
	assert_eq!(hinge.occluding, Some(true));
	assert_eq!(hinge.angle, None);
	assert_eq!(current.panes().len(), 2);
	assert_eq!(
		Posture::decode(&current.encode().unwrap()),
		Ok(current.clone())
	);

	// A stale posture returns immediately; the current one waits for a change.
	assert_eq!(
		web::wait_for_change(&Posture::default()).await,
		Ok(current.clone())
	);
	let mut waiting = Box::pin(web::wait_for_change(&current));
	assert!((&mut waiting).now_or_never().is_none());
	js_sys::Reflect::set(&posture, &"type".into(), &"continuous".into()).unwrap();
	posture
		.dispatch_event(&web_sys::Event::new("change").unwrap())
		.unwrap();
	let opened = waiting.await.unwrap();
	assert_eq!(opened.fold, Fold::Open);
	assert!(!opened.is_book());
}
