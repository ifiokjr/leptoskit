//! Cache Storage: named caches the page or a worker can open.

use crate::Result;
use crate::js;

/// Opens the named cache, creating it when missing.
///
/// # Errors
///
/// Outside a secure browser context.
pub async fn open(name: &str) -> Result<Cache> {
	#[cfg(target_arch = "wasm32")]
	{
		use wasm_bindgen::JsValue;
		let caches = js::caches()?;
		let inner = js::call_method(&caches, "open", &[JsValue::from_str(name)]).await?;
		Ok(Cache { inner })
	}
	#[cfg(not(target_arch = "wasm32"))]
	{
		let _ = name;
		js::unavailable_async("caches.open").await
	}
}

/// Whether a cache named `name` exists.
///
/// # Errors
///
/// Outside a secure browser context.
pub async fn has(name: &str) -> Result<bool> {
	#[cfg(target_arch = "wasm32")]
	{
		use wasm_bindgen::JsValue;
		let caches = js::caches()?;
		let value = js::call_method(&caches, "has", &[JsValue::from_str(name)]).await?;
		Ok(value.as_bool().unwrap_or(false))
	}
	#[cfg(not(target_arch = "wasm32"))]
	{
		let _ = name;
		js::unavailable_async("caches.has").await
	}
}

/// Deletes the cache named `name`. Returns whether it existed.
///
/// # Errors
///
/// Outside a secure browser context.
pub async fn delete(name: &str) -> Result<bool> {
	#[cfg(target_arch = "wasm32")]
	{
		use wasm_bindgen::JsValue;
		let caches = js::caches()?;
		let value = js::call_method(&caches, "delete", &[JsValue::from_str(name)]).await?;
		Ok(value.as_bool().unwrap_or(false))
	}
	#[cfg(not(target_arch = "wasm32"))]
	{
		let _ = name;
		js::unavailable_async("caches.delete").await
	}
}

/// The names of every cache this origin holds.
///
/// # Errors
///
/// Outside a secure browser context.
pub async fn keys() -> Result<Vec<String>> {
	#[cfg(target_arch = "wasm32")]
	{
		let caches = js::caches()?;
		let value = js::call_method(&caches, "keys", &[]).await?;
		js::from_js(&value)
	}
	#[cfg(not(target_arch = "wasm32"))]
	{
		js::unavailable_async("caches.keys").await
	}
}

/// One named cache.
#[derive(Debug, Clone)]
pub struct Cache {
	#[cfg(target_arch = "wasm32")]
	inner: wasm_bindgen::JsValue,
	#[cfg(not(target_arch = "wasm32"))]
	_private: (),
}

impl Cache {
	/// Adds `url` to the cache by fetching it.
	///
	/// # Errors
	///
	/// Outside a browser, or when the fetch fails.
	pub async fn add(&self, url: &str) -> Result<()> {
		#[cfg(target_arch = "wasm32")]
		{
			use wasm_bindgen::JsValue;
			js::call_method(&self.inner, "add", &[JsValue::from_str(url)])
				.await
				.map(drop)
		}
		#[cfg(not(target_arch = "wasm32"))]
		{
			let _ = url;
			js::unavailable_async("Cache.add").await
		}
	}

	/// Adds every URL in `urls` to the cache.
	///
	/// # Errors
	///
	/// Outside a browser, or when any fetch fails.
	pub async fn add_all(&self, urls: &[String]) -> Result<()> {
		#[cfg(target_arch = "wasm32")]
		{
			let list = js::to_js(&urls)?;
			js::call_method(&self.inner, "addAll", &[list])
				.await
				.map(drop)
		}
		#[cfg(not(target_arch = "wasm32"))]
		{
			let _ = urls;
			js::unavailable_async("Cache.addAll").await
		}
	}

	/// Whether the cache holds a response for `url`.
	///
	/// # Errors
	///
	/// Outside a browser.
	pub async fn has_url(&self, url: &str) -> Result<bool> {
		#[cfg(target_arch = "wasm32")]
		{
			use wasm_bindgen::JsValue;
			let value = js::call_method(&self.inner, "match", &[JsValue::from_str(url)]).await?;
			Ok(!value.is_undefined() && !value.is_null())
		}
		#[cfg(not(target_arch = "wasm32"))]
		{
			let _ = url;
			js::unavailable_async("Cache.match").await
		}
	}

	/// Removes the cached response for `url`. Returns whether one was removed.
	///
	/// # Errors
	///
	/// Outside a browser.
	pub async fn delete_url(&self, url: &str) -> Result<bool> {
		#[cfg(target_arch = "wasm32")]
		{
			use wasm_bindgen::JsValue;
			let value = js::call_method(&self.inner, "delete", &[JsValue::from_str(url)]).await?;
			Ok(value.as_bool().unwrap_or(false))
		}
		#[cfg(not(target_arch = "wasm32"))]
		{
			let _ = url;
			js::unavailable_async("Cache.delete").await
		}
	}

	/// The request URLs stored in this cache.
	///
	/// # Errors
	///
	/// Outside a browser.
	pub async fn urls(&self) -> Result<Vec<String>> {
		#[cfg(target_arch = "wasm32")]
		{
			let requests = js::call_method(&self.inner, "keys", &[]).await?;
			let list = js_sys::Array::from(&requests);
			let mut urls = Vec::with_capacity(list.length() as usize);
			for index in 0..list.length() {
				let request = list.get(index);
				let url = js_sys::Reflect::get(&request, &"url".into())
					.ok()
					.and_then(|value| value.as_string())
					.ok_or_else(|| crate::Error::Browser("cache request has no url".into()))?;
				urls.push(url);
			}
			Ok(urls)
		}
		#[cfg(not(target_arch = "wasm32"))]
		{
			js::unavailable_async("Cache.keys").await
		}
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
	fn cache_methods_name_their_apis_outside_a_browser() {
		let cache = Cache { _private: () };
		assert_eq!(
			block_on(cache.add("/assets/app.wasm")),
			Err(unavailable("Cache.add"))
		);
		assert_eq!(
			block_on(cache.add_all(&["/a".into(), "/b".into()])),
			Err(unavailable("Cache.addAll"))
		);
		assert_eq!(
			block_on(cache.has_url("/assets/app.wasm")),
			Err(unavailable("Cache.match"))
		);
		assert_eq!(
			block_on(cache.delete_url("/assets/app.wasm")),
			Err(unavailable("Cache.delete"))
		);
		assert_eq!(block_on(cache.urls()), Err(unavailable("Cache.keys")));
	}
}
