#![cfg(all(target_arch = "wasm32", target_os = "unknown", not(feature = "guest")))]

use std::task::Poll;

use futures::poll;
use leptoskit_bridge::Error;
use leptoskit_plugin_keep_awake::web;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

#[wasm_bindgen(inline_js = "
export function install() {
    const previous = Object.getOwnPropertyDescriptor(navigator, 'wakeLock');
    const state = { releases: 0, resolve: null, sentinel: null };
    Object.defineProperty(navigator, 'wakeLock', { configurable: true, value: {
        request() {
            const sentinel = new EventTarget();
            sentinel.released = false;
            sentinel.release = () => {
                state.releases++;
                sentinel.released = true;
                sentinel.dispatchEvent(new Event('release'));
                return Promise.resolve();
            };
            state.sentinel = sentinel;
            return new Promise(resolve => { state.resolve = () => resolve(sentinel); });
        }
    }});
    state.restore = () => {
        if (previous) Object.defineProperty(navigator, 'wakeLock', previous);
        else delete navigator.wakeLock;
    };
    state.revoke = () => {
        state.sentinel.released = true;
        state.sentinel.dispatchEvent(new Event('release'));
    };
    return state;
}
")]
extern "C" {
	fn install() -> Fixture;
	type Fixture;
	#[wasm_bindgen(method, structural)]
	fn restore(this: &Fixture);
	#[wasm_bindgen(method, structural)]
	fn resolve(this: &Fixture);
	#[wasm_bindgen(method, structural)]
	fn revoke(this: &Fixture);
	#[wasm_bindgen(method, structural, getter)]
	fn releases(this: &Fixture) -> u32;
}

struct Restore(Fixture);
impl Drop for Restore {
	fn drop(&mut self) {
		self.0.restore();
	}
}

async fn microtask() {
	JsFuture::from(js_sys::Promise::resolve(&JsValue::UNDEFINED))
		.await
		.unwrap();
}

#[wasm_bindgen_test]
async fn cancellation_cleans_up_late_acquisition_and_revocation_ends_a_live_lease() {
	let fixture = Restore(install());
	let mut acquiring = Box::pin(web::hold());
	assert!(matches!(poll!(acquiring.as_mut()), Poll::Pending));
	drop(acquiring);
	fixture.0.resolve();

	for _ in 0..20 {
		if fixture.0.releases() == 1 {
			break;
		}

		microtask().await;
	}

	assert_eq!(
		fixture.0.releases(),
		1,
		"late sentinel was not released after cancellation"
	);

	let mut active = Box::pin(web::hold());
	assert!(matches!(poll!(active.as_mut()), Poll::Pending));
	fixture.0.resolve();

	for _ in 0..5 {
		microtask().await;
	}

	assert!(matches!(poll!(active.as_mut()), Poll::Pending));
	fixture.0.revoke();
	assert_eq!(active.await, Err(Error::Cancelled));

	assert_eq!(
		fixture.0.releases(),
		1,
		"revoked sentinels should not release again"
	);

	let mut held = Box::pin(web::hold());
	assert!(matches!(poll!(held.as_mut()), Poll::Pending));
	fixture.0.resolve();

	for _ in 0..5 {
		microtask().await;
	}

	assert!(matches!(poll!(held.as_mut()), Poll::Pending));
	drop(held);
	assert_eq!(fixture.0.releases(), 2, "a live lease releases on drop");
}
