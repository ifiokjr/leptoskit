//! Calling browser APIs that live on `navigator` and `caches`. Values cross
//! as JSON when they are data; object handles stay as JavaScript values.

#[cfg(target_arch = "wasm32")]
pub(crate) use imp::*;

use crate::Error;

#[cfg(target_arch = "wasm32")]
mod imp {
	use js_sys::Function;
	use js_sys::JSON;
	use js_sys::Promise;
	use js_sys::Reflect;
	use serde::Serialize;
	use serde::de::DeserializeOwned;
	use wasm_bindgen::JsCast;
	use wasm_bindgen::JsValue;
	use wasm_bindgen::closure::Closure;
	use wasm_bindgen_futures::JsFuture;

	use super::*;

	/// The message of a JavaScript error.
	pub(crate) fn message(error: &JsValue) -> String {
		Reflect::get(error, &JsValue::from_str("message"))
			.ok()
			.and_then(|message| message.as_string())
			.or_else(|| error.as_string())
			.unwrap_or_else(|| "the browser refused".to_owned())
	}

	/// A serde value as a JavaScript value.
	pub(crate) fn to_js(value: &impl Serialize) -> Result<JsValue, Error> {
		let json = serde_json::to_string(value).map_err(|error| Error::Json(error.to_string()))?;
		JSON::parse(&json).map_err(|error| Error::Json(message(&error)))
	}

	/// A JavaScript value as a serde value. `undefined` reads as `null`.
	pub(crate) fn from_js<T: DeserializeOwned>(value: &JsValue) -> Result<T, Error> {
		let json = if value.is_undefined() {
			"null".to_owned()
		} else {
			JSON::stringify(value)
				.map_err(|error| Error::Json(message(&error)))?
				.as_string()
				.unwrap_or_else(|| "null".to_owned())
		};

		serde_json::from_str(&json).map_err(|error| Error::Json(error.to_string()))
	}

	/// Resolves a thenable to a JavaScript value.
	pub(crate) async fn await_js(value: JsValue) -> Result<JsValue, Error> {
		match value.dyn_into::<Promise>() {
			Ok(promise) => {
				JsFuture::from(promise)
					.await
					.map_err(|error| Error::Browser(message(&error)))
			}
			Err(value) => Ok(value),
		}
	}

	/// `navigator.serviceWorker`, when the page is a secure context that
	/// exposes it.
	pub(crate) fn container() -> Result<JsValue, Error> {
		let global = js_sys::global();
		let navigator = Reflect::get(&global, &JsValue::from_str("navigator"))
			.ok()
			.filter(|value| value.is_object())
			.ok_or_else(|| Error::Unavailable("navigator.serviceWorker".into()))?;
		Reflect::get(&navigator, &JsValue::from_str("serviceWorker"))
			.ok()
			.filter(|value| !value.is_undefined() && !value.is_null())
			.ok_or_else(|| Error::Unavailable("navigator.serviceWorker".into()))
	}

	/// `caches`, the Cache Storage root.
	pub(crate) fn caches() -> Result<JsValue, Error> {
		let global = js_sys::global();
		Reflect::get(&global, &JsValue::from_str("caches"))
			.ok()
			.filter(|value| !value.is_undefined() && !value.is_null())
			.ok_or_else(|| Error::Unavailable("caches".into()))
	}

	/// Calls a method on `this` and waits for its promise.
	pub(crate) async fn call_method(
		this: &JsValue,
		name: &str,
		args: &[JsValue],
	) -> Result<JsValue, Error> {
		let function = Reflect::get(this, &JsValue::from_str(name))
			.ok()
			.and_then(|value| value.dyn_into::<Function>().ok())
			.ok_or_else(|| Error::Unavailable(name.into()))?;
		let args: js_sys::Array = args.iter().collect();
		let result = function
			.apply(this, &args)
			.map_err(|error| Error::Browser(message(&error)))?;
		await_js(result).await
	}

	/// Reads a property from `this`.
	pub(crate) fn property(this: &JsValue, name: &str) -> Result<JsValue, Error> {
		Reflect::get(this, &JsValue::from_str(name))
			.map_err(|error| Error::Browser(message(&error)))
	}

	/// A listener removed when dropped.
	pub struct Listener {
		target: JsValue,
		remove_name: &'static str,
		function: Function,
		handler: Option<Closure<dyn FnMut(JsValue)>>,
	}

	impl Drop for Listener {
		fn drop(&mut self) {
			if let Ok(remove) = Reflect::get(&self.target, &JsValue::from_str(self.remove_name))
				.and_then(JsCast::dyn_into::<Function>)
			{
				let _ = remove.call1(&self.target, &self.function);
			}
			drop(self.handler.take());
		}
	}

	/// Listens with `addEventListener` / `removeEventListener`.
	pub(crate) fn listen_event(
		target: &JsValue,
		event: &str,
		handler: impl FnMut(JsValue) + 'static,
	) -> Result<Listener, Error> {
		let closure = Closure::<dyn FnMut(JsValue)>::new(handler);
		let function: Function = closure.as_ref().clone().unchecked_into();
		let add = Reflect::get(target, &JsValue::from_str("addEventListener"))
			.ok()
			.and_then(|value| value.dyn_into::<Function>().ok())
			.ok_or_else(|| Error::Unavailable("addEventListener".into()))?;
		add.call2(target, &JsValue::from_str(event), &function)
			.map_err(|error| Error::Browser(message(&error)))?;
		Ok(Listener {
			target: target.clone(),
			remove_name: "removeEventListener",
			function,
			handler: Some(closure),
		})
	}

	/// Listens with an extension-style `addListener` / `removeListener`.
	pub(crate) fn listen_callback(
		event: &JsValue,
		handler: impl FnMut(JsValue) + 'static,
	) -> Result<Listener, Error> {
		let closure = Closure::<dyn FnMut(JsValue)>::new(handler);
		let function: Function = closure.as_ref().clone().unchecked_into();
		let add = Reflect::get(event, &JsValue::from_str("addListener"))
			.ok()
			.and_then(|value| value.dyn_into::<Function>().ok())
			.ok_or_else(|| Error::Unavailable("addListener".into()))?;
		add.call1(event, &function)
			.map_err(|error| Error::Browser(message(&error)))?;
		Ok(Listener {
			target: event.clone(),
			remove_name: "removeListener",
			function,
			handler: Some(closure),
		})
	}
}

#[cfg(not(target_arch = "wasm32"))]
#[allow(clippy::unused_async, reason = "matches the browser signature")]
pub(crate) async fn unavailable_async<T>(path: &str) -> Result<T, Error> {
	Err(Error::Unavailable(path.to_owned()))
}

/// Outside a browser, listening fails as unavailable.
#[cfg(not(target_arch = "wasm32"))]
pub struct Listener;
