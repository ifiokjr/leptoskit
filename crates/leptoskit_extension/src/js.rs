//! Calling the extension API: `browser` where it exists (Firefox and
//! Safari), `chrome` otherwise. Values cross as JSON, so every argument and
//! answer is a serde type.

#[cfg(target_arch = "wasm32")]
pub use imp::Listener;
#[cfg(target_arch = "wasm32")]
pub(crate) use imp::*;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::Error;

#[cfg(target_arch = "wasm32")]
mod imp {
	use js_sys::Array;
	use js_sys::Function;
	use js_sys::JSON;
	use js_sys::Promise;
	use js_sys::Reflect;
	use wasm_bindgen::JsCast;
	use wasm_bindgen::JsValue;
	use wasm_bindgen::closure::Closure;
	use wasm_bindgen_futures::JsFuture;

	use super::*;

	/// The extension API's root object.
	pub(crate) fn namespace() -> Option<JsValue> {
		let global = js_sys::global();
		for name in ["browser", "chrome"] {
			let candidate = Reflect::get(&global, &JsValue::from_str(name)).ok()?;
			if candidate.is_object()
				&& Reflect::get(&candidate, &JsValue::from_str("runtime"))
					.is_ok_and(|runtime| runtime.is_object())
			{
				return Some(candidate);
			}
		}
		None
	}

	/// The object at `path` under the namespace, such as `tabs`.
	pub(crate) fn object(path: &str) -> Result<JsValue, Error> {
		let mut value = namespace().ok_or_else(|| Error::Unavailable(path.to_owned()))?;
		for part in path.split('.') {
			value = Reflect::get(&value, &JsValue::from_str(part))
				.ok()
				.filter(|value| !value.is_undefined() && !value.is_null())
				.ok_or_else(|| Error::Unavailable(path.to_owned()))?;
		}
		Ok(value)
	}

	/// Whether the API at `path` exists in this browser.
	#[must_use]
	pub(crate) fn has(path: &str) -> bool {
		object(path).is_ok()
	}

	fn function(path: &str) -> Result<(JsValue, Function), Error> {
		let (parent, name) = path.rsplit_once('.').unwrap_or(("", path));
		let this = if parent.is_empty() {
			namespace().ok_or_else(|| Error::Unavailable(path.to_owned()))?
		} else {
			object(parent)?
		};
		let function = Reflect::get(&this, &JsValue::from_str(name))
			.ok()
			.and_then(|value| value.dyn_into::<Function>().ok())
			.ok_or_else(|| Error::Unavailable(path.to_owned()))?;
		Ok((this, function))
	}

	/// The message of a JavaScript error.
	pub(crate) fn message(error: &JsValue) -> String {
		Reflect::get(error, &JsValue::from_str("message"))
			.ok()
			.and_then(|message| message.as_string())
			.or_else(|| error.as_string())
			.unwrap_or_else(|| "the browser refused".to_owned())
	}

	/// Calls the API function at `path` and waits for its promise.
	pub(crate) async fn call_raw(path: &str, args: &[JsValue]) -> Result<JsValue, Error> {
		let (this, function) = function(path)?;
		let args: Array = args.iter().collect();
		let result = function
			.apply(&this, &args)
			.map_err(|error| Error::Browser(message(&error)))?;
		match result.dyn_into::<Promise>() {
			Ok(promise) => {
				JsFuture::from(promise)
					.await
					.map_err(|error| Error::Browser(message(&error)))
			}
			Err(value) => Ok(value),
		}
	}

	/// Calls the API function at `path` with `args` and a callback, and
	/// waits for the callback's first two arguments. Chromium's developer
	/// tools APIs answer only through a callback; Firefox calls it too.
	pub(crate) async fn call_back(
		path: &str,
		args: &[JsValue],
	) -> Result<(JsValue, JsValue), Error> {
		let (this, function) = function(path)?;
		let promise = Promise::new(&mut |resolve, reject| {
			let answered = Closure::once_into_js(move |first: JsValue, second: JsValue| {
				let _ = resolve.call1(&JsValue::NULL, &Array::of2(&first, &second));
			});
			let args: Array = args.iter().chain([&answered]).collect();
			if let Err(error) = function.apply(&this, &args) {
				let _ = reject.call1(&JsValue::NULL, &error);
			}
		});
		let answer: Array = JsFuture::from(promise)
			.await
			.map_err(|error| Error::Browser(message(&error)))?
			.unchecked_into();
		Ok((answer.get(0), answer.get(1)))
	}

	/// The value at `path` under the namespace, as a serde value.
	pub(crate) fn property<T: DeserializeOwned>(path: &str) -> Result<T, Error> {
		from_js(&object(path)?)
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

	/// Calls `path` with serde arguments and reads its answer.
	pub(crate) async fn call<T: DeserializeOwned>(
		path: &str,
		args: &[&dyn ErasedSerialize],
	) -> Result<T, Error> {
		let args = args
			.iter()
			.map(|arg| arg.to_js())
			.collect::<Result<Vec<_>, _>>()?;
		from_js(&call_raw(path, &args).await?)
	}

	type Handler = Closure<dyn FnMut(JsValue, JsValue, JsValue) -> JsValue>;

	/// A listener on an extension event, removed when dropped.
	pub struct Listener {
		event: JsValue,
		function: Function,
		// Keeps the Rust handler alive while the browser can call it.
		handler: Option<Handler>,
	}

	impl Drop for Listener {
		fn drop(&mut self) {
			if let Ok(remove) = Reflect::get(&self.event, &JsValue::from_str("removeListener"))
				.and_then(JsCast::dyn_into::<Function>)
			{
				let _ = remove.call1(&self.event, &self.function);
			}
			// Only now can the handler go: the browser no longer calls it.
			drop(self.handler.take());
		}
	}

	/// Listens to the event at `path`, such as `storage.onChanged`.
	pub(crate) fn listen(
		path: &str,
		handler: impl FnMut(JsValue, JsValue, JsValue) -> JsValue + 'static,
	) -> Result<Listener, Error> {
		let event = object(path)?;
		let closure = Closure::<dyn FnMut(JsValue, JsValue, JsValue) -> JsValue>::new(handler);
		let function: Function = closure.as_ref().clone().unchecked_into();
		let add = Reflect::get(&event, &JsValue::from_str("addListener"))
			.ok()
			.and_then(|value| value.dyn_into::<Function>().ok())
			.ok_or_else(|| Error::Unavailable(path.to_owned()))?;
		add.call1(&event, &function)
			.map_err(|error| Error::Browser(message(&error)))?;
		Ok(Listener {
			event,
			function,
			handler: Some(closure),
		})
	}
}

/// A serde value that can become a JavaScript value, for argument lists of
/// mixed types.
pub(crate) trait ErasedSerialize {
	#[cfg(target_arch = "wasm32")]
	fn to_js(&self) -> Result<wasm_bindgen::JsValue, Error>;
}

impl<T: Serialize> ErasedSerialize for T {
	#[cfg(target_arch = "wasm32")]
	fn to_js(&self) -> Result<wasm_bindgen::JsValue, Error> {
		to_js(self)
	}
}

/// Outside a browser every API is unavailable, so the app's code compiles,
/// and its tests run, on any host.
#[cfg(not(target_arch = "wasm32"))]
#[allow(clippy::unused_async, reason = "the same signature as in a browser")]
pub(crate) async fn call<T: DeserializeOwned>(
	path: &str,
	_args: &[&dyn ErasedSerialize],
) -> Result<T, Error> {
	Err(Error::Unavailable(path.to_owned()))
}

/// Whether the API at `path` exists: never, outside a browser.
#[cfg(not(target_arch = "wasm32"))]
#[must_use]
pub(crate) fn has(_path: &str) -> bool {
	false
}

/// Outside a browser there's no value to read.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn property<T: DeserializeOwned>(path: &str) -> Result<T, Error> {
	Err(Error::Unavailable(path.to_owned()))
}

/// Outside a browser, listening fails as unavailable.
#[cfg(not(target_arch = "wasm32"))]
pub struct Listener;
