#![cfg(all(target_arch = "wasm32", target_os = "unknown", not(feature = "guest")))]

use std::task::Poll;

use futures::poll;
use leptoskit_bridge::Error;
use leptoskit_plugin_screen_orientation::Orientation;
use leptoskit_plugin_screen_orientation::Policy;
use leptoskit_plugin_screen_orientation::web;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

#[wasm_bindgen(inline_js = "
export function install() {
    const previous = Object.getOwnPropertyDescriptor(screen, 'orientation');
    const state = { unlocks: 0, error: null, pending: false, requested: null, listeners: 0 };
    const orientation = Object.create(ScreenOrientation.prototype);
    Object.defineProperties(orientation, {
        type: { value: 'landscape-primary' }, angle: { value: 90 },
        lock: { value: policy => {
            state.requested = policy;
            if (state.error) return Promise.reject(new DOMException('Test rejection', \
                            state.error));
            return new Promise((resolve, reject) => { state.resolve = resolve; state.reject = \
                            reject; state.pending = true; });
        }},
        unlock: { value: () => {
            state.unlocks++;
            if (state.pending) { state.pending = false; state.reject(new DOMException('Unlocked', \
                            'AbortError')); }
        }}
    });
    Object.defineProperty(screen, 'orientation', { configurable: true, value: orientation });
    const add = EventTarget.prototype.addEventListener;
    const remove = EventTarget.prototype.removeEventListener;
    const names = new Set(['visibilitychange', 'fullscreenchange', 'pagehide']);
    EventTarget.prototype.addEventListener = function(name, callback, options) {
        if (names.has(name)) state.listeners++;
        return add.call(this, name, callback, options);
    };
    EventTarget.prototype.removeEventListener = function(name, callback, options) {
        if (names.has(name)) state.listeners--;
        return remove.call(this, name, callback, options);
    };
    state.accept = () => { state.pending = false; state.resolve(); };
    state.hide = () => window.dispatchEvent(new Event('pagehide'));
    state.restore = () => {
        if (previous) Object.defineProperty(screen, 'orientation', previous); else delete \
                            screen.orientation;
        EventTarget.prototype.addEventListener = add;
        EventTarget.prototype.removeEventListener = remove;
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
	fn accept(this: &Fixture);
	#[wasm_bindgen(method, structural)]
	fn hide(this: &Fixture);
	#[wasm_bindgen(method, structural, getter)]
	fn unlocks(this: &Fixture) -> u32;
	#[wasm_bindgen(method, structural, getter)]
	fn listeners(this: &Fixture) -> u32;
	#[wasm_bindgen(method, structural, getter)]
	fn requested(this: &Fixture) -> String;
	#[wasm_bindgen(method, structural, setter = error)]
	fn set_error(this: &Fixture, error: &str);
}
struct Restore(Fixture);
impl Drop for Restore {
	fn drop(&mut self) {
		self.0.restore();
	}
}
async fn tick() {
	JsFuture::from(js_sys::Promise::resolve(&JsValue::UNDEFINED))
		.await
		.unwrap();
}

#[wasm_bindgen_test]
async fn browser_policy_lifetime_cancellation_and_errors() {
	let fixture = Restore(install());
	let snapshot = web::snapshot().unwrap();
	assert_eq!(snapshot.orientation, Orientation::Landscape);
	assert_eq!(snapshot.angle, Some(90));
	let mut first = Box::pin(web::hold(Policy::Portrait));
	assert!(matches!(poll!(first.as_mut()), Poll::Pending));
	assert_eq!(fixture.0.requested(), "portrait");
	assert_eq!(fixture.0.listeners(), 3);

	assert_eq!(web::hold(Policy::Any).await, Err(Error::Busy));
	drop(first);
	tick().await;
	assert_eq!(fixture.0.unlocks(), 1);
	assert_eq!(fixture.0.listeners(), 0);
	let mut second = Box::pin(web::hold(Policy::LandscapeSecondary));
	assert!(matches!(poll!(second.as_mut()), Poll::Pending));
	fixture.0.accept();

	tick().await;
	assert!(matches!(poll!(second.as_mut()), Poll::Pending));
	fixture.0.hide();
	assert_eq!(second.await, Err(Error::Cancelled));
	assert_eq!(fixture.0.unlocks(), 2);
	assert_eq!(fixture.0.listeners(), 0);
	for (name, expected) in [
		("SecurityError", Error::PermissionDenied),
		("NotAllowedError", Error::PermissionDenied),
		("NotSupportedError", Error::Unavailable),
		("AbortError", Error::Cancelled),
		("OperationError", Error::Native),
	] {
		fixture.0.set_error(name);
		assert_eq!(web::hold(Policy::Any).await, Err(expected));
		assert_eq!(fixture.0.listeners(), 0);
	}
}
