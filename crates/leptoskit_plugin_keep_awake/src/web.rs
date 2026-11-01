//! Screen Wake Lock in secure, visible browser contexts. A visibility change or
//! OS revocation ends the holding future; reacquisition must be explicit.

use std::convert::Infallible;

use futures::channel::oneshot;
use gloo_events::EventListener;
use leptoskit_bridge::Error;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use wasm_bindgen_futures::spawn_local;

#[wasm_bindgen]
extern "C" {
	#[wasm_bindgen(extends = js_sys::Object)]
	type WakeLock;
	#[wasm_bindgen(method, structural, catch)]
	fn request(this: &WakeLock, kind: &str) -> Result<js_sys::Promise, JsValue>;
	#[wasm_bindgen(extends = web_sys::EventTarget, extends = js_sys::Object)]
	type WakeLockSentinel;
	#[wasm_bindgen(method, structural, getter)]
	fn released(this: &WakeLockSentinel) -> bool;
	#[wasm_bindgen(method, structural, catch)]
	fn release(this: &WakeLockSentinel) -> Result<js_sys::Promise, JsValue>;
}

struct Guard(WakeLockSentinel);
impl Drop for Guard {
	fn drop(&mut self) {
		if self.0.released() {
			return;
		}

		match self.0.release() {
			Ok(promise) => {
				spawn_local(async move {
					if let Err(error) = JsFuture::from(promise).await {
						web_sys::console::error_1(&error);
					}
				});
			}
			Err(error) => web_sys::console::error_1(&error),
		}
	}
}

fn map_error(error: &JsValue) -> Error {
	if let Some(exception) = error.dyn_ref::<web_sys::DomException>() {
		match exception.name().as_str() {
			"NotAllowedError" | "SecurityError" => return Error::PermissionDenied,
			"NotSupportedError" => return Error::Unavailable,
			_ => {}
		}
	}

	Error::Native
}

/// Holds the display awake until cancellation or OS release. Poll alongside
/// application work; this never reports successful completion. Dropping during
/// acquisition also releases a sentinel that arrives after cancellation.
pub async fn hold() -> Result<Infallible, Error> {
	let navigator = web_sys::window().ok_or(Error::Unavailable)?.navigator();
	let value = js_sys::Reflect::get(navigator.as_ref(), &JsValue::from_str("wakeLock"))
		.map_err(|error| map_error(&error))?;

	if value.is_null() || value.is_undefined() {
		return Err(Error::Unavailable);
	}

	let wake_lock: WakeLock = value.unchecked_into();
	let promise = wake_lock
		.request("screen")
		.map_err(|error| map_error(&error))?;
	let (sender, receiver) = oneshot::channel();
	// JS acquisition cannot be cancelled. Keep only this bounded completion
	// task alive, so cancellation drops and releases a late-arriving native
	// sentinel.
	spawn_local(async move {
		let result = JsFuture::from(promise)
			.await
			.map(|value| Guard(value.unchecked_into::<WakeLockSentinel>()))
			.map_err(|error| map_error(&error));
		drop(sender.send(result));
	});

	let guard = receiver.await.map_err(|_| Error::Cancelled)??;
	let (sender, receiver) = oneshot::channel();
	let mut sender = Some(sender);
	let listener = EventListener::new(guard.0.as_ref(), "release", move |_| {
		if let Some(sender) = sender.take() {
			let _ = sender.send(());
		}
	});

	if guard.0.released() {
		return Err(Error::Cancelled);
	}

	let _ = receiver.await;
	drop(listener);
	drop(guard);
	Err(Error::Cancelled)
}
