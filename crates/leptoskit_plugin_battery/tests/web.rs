#![cfg(all(target_arch = "wasm32", not(feature = "guest")))]

use std::future::Future;
use std::pin::Pin;
use std::task::Context;
use std::task::Poll;
use std::task::Waker;

use leptoskit_bridge::Error;
use leptoskit_plugin_battery::State;
use leptoskit_plugin_battery::web;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

// Real DOM EventTarget dispatch with controlled battery readings. Instrumenting
// add/remove proves dropping a suspended Rust future detaches its JS callbacks.
#[wasm_bindgen(inline_js = r"
let original;
let battery;
let active;
export function installBatteryFixture() {
  original = Object.getOwnPropertyDescriptor(navigator, 'getBattery');
  battery = new EventTarget();
  Object.setPrototypeOf(battery, BatteryManager.prototype);
  active = new Map();
  Object.defineProperties(battery, {
    level: {value: 0.37, writable: true, configurable: true},
    charging: {value: false, writable: true, configurable: true},
    addEventListener: {value(name, callback) {
      active.set(name, callback);
      EventTarget.prototype.addEventListener.call(this, name, callback);
    }},
    removeEventListener: {value(name, callback) {
      if (active.get(name) === callback) active.delete(name);
      EventTarget.prototype.removeEventListener.call(this, name, callback);
    }}
  });
  Object.defineProperty(navigator, 'getBattery', {
    configurable: true, value() { return Promise.resolve(battery); }
  });
}
export function restoreBatteryFixture() {
  if (original) Object.defineProperty(navigator, 'getBattery', original);
  else delete navigator.getBattery;
}
export function batteryListeners() { return active.size; }
export function changeBatteryLevel(value) {
  battery.level = value;
  battery.dispatchEvent(new Event('levelchange'));
}
export function rejectBattery(name) {
  Object.defineProperty(navigator, 'getBattery', {
    configurable: true, value() { return Promise.reject(new DOMException('test', name)); }
  });
}
export function removeBattery() {
  Object.defineProperty(navigator, 'getBattery', {configurable: true, value: undefined});
}
")]
extern "C" {
	#[wasm_bindgen(js_name = installBatteryFixture)]
	fn install_fixture();
	#[wasm_bindgen(js_name = restoreBatteryFixture)]
	fn restore_fixture();
	#[wasm_bindgen(js_name = batteryListeners)]
	fn listeners() -> u32;
	#[wasm_bindgen(js_name = changeBatteryLevel)]
	fn change_level(value: f64);
	#[wasm_bindgen(js_name = rejectBattery)]
	fn reject(name: &str);
	#[wasm_bindgen(js_name = removeBattery)]
	fn remove_api();
}

/// Battery levels the fake Browser Status API reports between waits.
const JUST_UNDER_HALF: f64 = 0.42;
const JUST_OVER_HALF: f64 = 0.51;

struct Fixture;

impl Fixture {
	fn new() -> Self {
		install_fixture();
		Self
	}
}

impl Drop for Fixture {
	fn drop(&mut self) {
		restore_fixture();
	}
}

async fn installed<F: Future>(mut pending: Pin<&mut F>) {
	for _ in 0..8 {
		assert!(
			pending
				.as_mut()
				.poll(&mut Context::from_waker(Waker::noop()))
				.is_pending()
		);

		if listeners() == 2 {
			return;
		}

		JsFuture::from(js_sys::Promise::resolve(&JsValue::UNDEFINED))
			.await
			.unwrap();
	}

	panic!("battery listeners were not installed");
}

#[wasm_bindgen_test]
async fn browser_waiters_detach_on_drop_and_completion_and_preserve_errors() {
	let _fixture = Fixture::new();
	let initial = web::snapshot().await.unwrap();
	assert_eq!(initial.charge.unwrap().thousandths(), 370);
	assert_eq!(initial.state, State::Discharging);

	let mut cancelled = Box::pin(web::wait_for_change(initial));
	installed(cancelled.as_mut()).await;
	drop(cancelled);
	assert_eq!(
		listeners(),
		0,
		"dropping the waiter removes both DOM listeners"
	);
	change_level(JUST_UNDER_HALF);
	let between_waits = web::wait_for_change(initial).await.unwrap();
	assert_eq!(between_waits.charge.unwrap().thousandths(), 420);
	assert_eq!(
		listeners(),
		0,
		"immediate completion removes both listeners"
	);

	let mut changed = Box::pin(web::wait_for_change(between_waits));
	installed(changed.as_mut()).await;
	change_level(JUST_OVER_HALF);
	let Poll::Ready(Ok(current)) = changed
		.as_mut()
		.poll(&mut Context::from_waker(Waker::noop()))
	else {
		panic!("the real levelchange DOM event must complete the waiter");
	};
	assert_eq!(current.charge.unwrap().thousandths(), 510);
	assert_eq!(listeners(), 0);

	reject("NotAllowedError");
	assert_eq!(web::snapshot().await, Err(Error::PermissionDenied));
	reject("SecurityError");
	assert_eq!(web::snapshot().await, Err(Error::PermissionDenied));
	reject("UnknownError");
	assert_eq!(web::snapshot().await, Err(Error::Native));
	remove_api();
	assert_eq!(web::snapshot().await, Err(Error::Unavailable));
}
