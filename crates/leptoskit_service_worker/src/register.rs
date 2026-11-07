//! Registering a service worker and reading who controls the page.

use serde::Deserialize;
use serde::Serialize;

use crate::Error;
use crate::Result;
use crate::js;
use crate::push::PushManager;

/// How the browser may reuse a previously cached worker script.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UpdateViaCache {
	/// Bypass HTTP cache for imported scripts only.
	Imports,
	/// Use the HTTP cache for the worker and its imports.
	All,
	/// Bypass the HTTP cache entirely.
	#[default]
	None,
}

/// Options for [`register`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegisterOptions {
	/// The scope under which the worker controls pages. Defaults to the
	/// directory of the script URL.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub scope: Option<String>,
	/// How the browser may reuse a previously cached worker script.
	#[serde(default, skip_serializing_if = "is_default_update")]
	pub update_via_cache: UpdateViaCache,
}

#[allow(
	clippy::trivially_copy_pass_by_ref,
	reason = "serde skip_serializing_if takes &T"
)]
fn is_default_update(value: &UpdateViaCache) -> bool {
	*value == UpdateViaCache::None
}

impl RegisterOptions {
	/// Registers under `scope`.
	#[must_use]
	pub fn scope(mut self, scope: impl Into<String>) -> Self {
		self.scope = Some(scope.into());
		self
	}

	/// Sets how the browser may reuse a previously cached worker script.
	#[must_use]
	pub const fn update_via_cache(mut self, update_via_cache: UpdateViaCache) -> Self {
		self.update_via_cache = update_via_cache;
		self
	}
}

/// A registered service worker.
#[derive(Debug, Clone)]
pub struct Registration {
	#[cfg(target_arch = "wasm32")]
	pub(crate) inner: wasm_bindgen::JsValue,
	#[cfg(not(target_arch = "wasm32"))]
	_private: (),
}

impl Registration {
	/// The scope URL this registration controls.
	///
	/// # Errors
	///
	/// Outside a browser, or when the registration has no scope.
	pub fn scope(&self) -> Result<String> {
		#[cfg(target_arch = "wasm32")]
		{
			js::property(&self.inner, "scope")?
				.as_string()
				.ok_or_else(|| Error::Browser("registration has no scope".into()))
		}
		#[cfg(not(target_arch = "wasm32"))]
		{
			Err(Error::Unavailable("ServiceWorkerRegistration.scope".into()))
		}
	}

	/// Asks the browser to check for an updated worker script.
	///
	/// # Errors
	///
	/// Outside a browser, or when the browser refuses.
	pub async fn update(&self) -> Result<()> {
		#[cfg(target_arch = "wasm32")]
		{
			js::call_method(&self.inner, "update", &[]).await.map(drop)
		}
		#[cfg(not(target_arch = "wasm32"))]
		{
			js::unavailable_async("ServiceWorkerRegistration.update").await
		}
	}

	/// Unregisters this worker. Pages already controlled stay controlled until
	/// they reload.
	///
	/// # Errors
	///
	/// Outside a browser.
	pub async fn unregister(&self) -> Result<bool> {
		#[cfg(target_arch = "wasm32")]
		{
			let value = js::call_method(&self.inner, "unregister", &[]).await?;
			Ok(value.as_bool().unwrap_or(false))
		}
		#[cfg(not(target_arch = "wasm32"))]
		{
			js::unavailable_async("ServiceWorkerRegistration.unregister").await
		}
	}

	/// The active worker, when one is controlling pages in this scope.
	///
	/// # Errors
	///
	/// Outside a browser.
	pub fn active(&self) -> Result<Option<Worker>> {
		#[cfg(target_arch = "wasm32")]
		{
			let value = js::property(&self.inner, "active")?;
			Ok((!value.is_null() && !value.is_undefined()).then(|| Worker { inner: value }))
		}
		#[cfg(not(target_arch = "wasm32"))]
		{
			Err(Error::Unavailable(
				"ServiceWorkerRegistration.active".into(),
			))
		}
	}

	/// The installing worker, when an update is installing.
	///
	/// # Errors
	///
	/// Outside a browser.
	pub fn installing(&self) -> Result<Option<Worker>> {
		#[cfg(target_arch = "wasm32")]
		{
			let value = js::property(&self.inner, "installing")?;
			Ok((!value.is_null() && !value.is_undefined()).then(|| Worker { inner: value }))
		}
		#[cfg(not(target_arch = "wasm32"))]
		{
			Err(Error::Unavailable(
				"ServiceWorkerRegistration.installing".into(),
			))
		}
	}

	/// The waiting worker, when an update is installed but not yet active.
	///
	/// # Errors
	///
	/// Outside a browser.
	pub fn waiting(&self) -> Result<Option<Worker>> {
		#[cfg(target_arch = "wasm32")]
		{
			let value = js::property(&self.inner, "waiting")?;
			Ok((!value.is_null() && !value.is_undefined()).then(|| Worker { inner: value }))
		}
		#[cfg(not(target_arch = "wasm32"))]
		{
			Err(Error::Unavailable(
				"ServiceWorkerRegistration.waiting".into(),
			))
		}
	}

	/// The push manager for this registration.
	///
	/// # Errors
	///
	/// Outside a browser, or when Push is unavailable.
	pub fn push_manager(&self) -> Result<PushManager> {
		#[cfg(target_arch = "wasm32")]
		{
			let value = js::property(&self.inner, "pushManager")?;
			if value.is_undefined() || value.is_null() {
				return Err(Error::Unavailable("PushManager".into()));
			}
			Ok(PushManager { inner: value })
		}
		#[cfg(not(target_arch = "wasm32"))]
		{
			Err(Error::Unavailable("PushManager".into()))
		}
	}
}

/// A service worker instance (installing, waiting, or active).
#[derive(Debug, Clone)]
pub struct Worker {
	#[cfg(target_arch = "wasm32")]
	pub(crate) inner: wasm_bindgen::JsValue,
	#[cfg(not(target_arch = "wasm32"))]
	pub(crate) _private: (),
}

impl Worker {
	/// Posts a structured-cloneable message to this worker.
	///
	/// # Errors
	///
	/// Outside a browser, or when the worker refuses the value.
	pub fn post_message<M: Serialize>(&self, message: &M) -> Result<()> {
		#[cfg(target_arch = "wasm32")]
		{
			use wasm_bindgen::JsCast;
			let function = js_sys::Reflect::get(&self.inner, &"postMessage".into())
				.ok()
				.and_then(|value| value.dyn_into::<js_sys::Function>().ok())
				.ok_or_else(|| Error::Unavailable("ServiceWorker.postMessage".into()))?;
			function
				.call1(&self.inner, &js::to_js(message)?)
				.map_err(|error| Error::Browser(js::message(&error)))
				.map(drop)
		}
		#[cfg(not(target_arch = "wasm32"))]
		{
			let _ = message;
			Err(Error::Unavailable("ServiceWorker.postMessage".into()))
		}
	}
}

/// Registers the service worker at `script_url`. The script must be served
/// from the same origin, typically from your web assets (for example
/// `/sw.js`).
///
/// # Errors
///
/// Outside a secure browser context, or when the browser refuses the script.
pub async fn register(script_url: &str, options: RegisterOptions) -> Result<Registration> {
	#[cfg(target_arch = "wasm32")]
	{
		use wasm_bindgen::JsValue;
		let container = js::container()?;
		let mut args = vec![JsValue::from_str(script_url)];
		if options.scope.is_some() || options.update_via_cache != UpdateViaCache::None {
			args.push(js::to_js(&options)?);
		}
		let inner = js::call_method(&container, "register", &args).await?;
		Ok(Registration { inner })
	}
	#[cfg(not(target_arch = "wasm32"))]
	{
		let _ = (script_url, options);
		js::unavailable_async("navigator.serviceWorker.register").await
	}
}

/// Waits until a service worker is active and ready to control clients.
///
/// # Errors
///
/// Outside a secure browser context.
pub async fn ready() -> Result<Registration> {
	#[cfg(target_arch = "wasm32")]
	{
		let container = js::container()?;
		let promise = js::property(&container, "ready")?;
		let inner = js::await_js(promise).await?;
		Ok(Registration { inner })
	}
	#[cfg(not(target_arch = "wasm32"))]
	{
		js::unavailable_async("navigator.serviceWorker.ready").await
	}
}

/// The registration for `scope`, or the one that matches this page when
/// `scope` is `None`.
///
/// # Errors
///
/// Outside a secure browser context.
pub async fn get_registration(scope: Option<&str>) -> Result<Option<Registration>> {
	#[cfg(target_arch = "wasm32")]
	{
		use wasm_bindgen::JsValue;
		let container = js::container()?;
		let args = match scope {
			Some(scope) => vec![JsValue::from_str(scope)],
			None => Vec::new(),
		};
		let value = js::call_method(&container, "getRegistration", &args).await?;
		Ok((!value.is_undefined() && !value.is_null()).then(|| Registration { inner: value }))
	}
	#[cfg(not(target_arch = "wasm32"))]
	{
		let _ = scope;
		js::unavailable_async("navigator.serviceWorker.getRegistration").await
	}
}

/// The worker controlling this page, if any.
///
/// # Errors
///
/// Outside a secure browser context.
pub fn controller() -> Result<Option<Worker>> {
	#[cfg(target_arch = "wasm32")]
	{
		let container = js::container()?;
		let value = js::property(&container, "controller")?;
		Ok((!value.is_null() && !value.is_undefined()).then(|| Worker { inner: value }))
	}
	#[cfg(not(target_arch = "wasm32"))]
	{
		// Match a page with no controlling worker: `Ok(None)` rather than a
		// hard unavailable, so callers can distinguish "no controller" from
		// other browser failures the same way they do in a browser.
		Ok(None)
	}
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
	use futures::executor::block_on;

	use super::*;
	use crate::Error;

	fn unavailable(path: &str) -> Error {
		Error::Unavailable(path.to_owned())
	}

	#[test]
	fn register_options_round_trip() {
		let options = RegisterOptions::default()
			.scope("/app/")
			.update_via_cache(UpdateViaCache::All);
		assert_eq!(options.scope.as_deref(), Some("/app/"));
		assert_eq!(options.update_via_cache, UpdateViaCache::All);
		let json = serde_json::to_value(&options).expect("options serialize");
		assert_eq!(json["scope"], "/app/");
		assert_eq!(json["update_via_cache"], "all");
		// Default update_via_cache is skipped so is_default_update runs both
		// ways.
		let default = serde_json::to_value(RegisterOptions::default()).expect("default serialize");
		assert!(default.get("update_via_cache").is_none());
		assert!(is_default_update(&UpdateViaCache::None));
		assert!(!is_default_update(&UpdateViaCache::Imports));
	}

	#[test]
	fn registration_and_worker_name_their_apis_outside_a_browser() {
		let registration = Registration { _private: () };
		assert_eq!(
			registration.scope(),
			Err(unavailable("ServiceWorkerRegistration.scope"))
		);
		assert_eq!(
			block_on(registration.update()),
			Err(unavailable("ServiceWorkerRegistration.update"))
		);
		assert_eq!(
			block_on(registration.unregister()),
			Err(unavailable("ServiceWorkerRegistration.unregister"))
		);
		assert!(matches!(
			registration.active(),
			Err(error) if error == unavailable("ServiceWorkerRegistration.active")
		));
		assert!(matches!(
			registration.installing(),
			Err(error) if error == unavailable("ServiceWorkerRegistration.installing")
		));
		assert!(matches!(
			registration.waiting(),
			Err(error) if error == unavailable("ServiceWorkerRegistration.waiting")
		));
		assert!(matches!(
			registration.push_manager(),
			Err(error) if error == unavailable("PushManager")
		));

		let worker = Worker { _private: () };
		assert_eq!(
			worker.post_message(&"ping"),
			Err(unavailable("ServiceWorker.postMessage"))
		);
	}
}
