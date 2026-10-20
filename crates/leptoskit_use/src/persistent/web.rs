use std::marker::PhantomData;

use serde::Serialize;
use serde::de::DeserializeOwned;
use wasm_bindgen::JsValue;

use crate::AsyncStorage;
use crate::Storage;
use crate::StorageError;
use crate::StorageFuture;

/// Namespaced JSON persistence in the browser's local storage.
///
/// Construct this in the browser host, then use the same storage hooks as a
/// native application. Access and quota errors propagate. Async operations are
/// immediately ready because browser local storage is synchronous. For truly
/// asynchronous `IndexedDB` or worker storage, implement [`AsyncStorage`].
/// No JavaScript handles are kept in shared Rust state; access is resolved at
/// the host boundary on the calling browser thread.
#[derive(Debug, Clone)]
pub struct WebStorage<T> {
	namespace: String,
	value: PhantomData<fn() -> T>,
}

impl<T> WebStorage<T> {
	/// Open a namespace, returning an error if local storage is inaccessible.
	pub fn new(namespace: impl Into<String>) -> Result<Self, StorageError> {
		storage()?;
		Ok(Self {
			namespace: namespace.into(),
			value: PhantomData,
		})
	}

	fn key(&self, key: &str) -> String {
		format!("{}:{}{key}", self.namespace.len(), self.namespace)
	}
}

fn storage() -> Result<web_sys::Storage, StorageError> {
	web_sys::window()
		.ok_or_else(|| StorageError::Unavailable("no browser window".into()))?
		.local_storage()
		.map_err(|error| browser_error(&error))?
		.ok_or_else(|| StorageError::Unavailable("local storage is disabled".into()))
}

fn browser_error(error: &JsValue) -> StorageError {
	StorageError::Browser(format!("{error:?}"))
}

impl<T> Storage<T> for WebStorage<T>
where
	T: Serialize + DeserializeOwned + Send + Sync + 'static,
{
	type Error = StorageError;

	fn get(&self, key: &str) -> Result<Option<T>, StorageError> {
		storage()?
			.get_item(&self.key(key))
			.map_err(|error| browser_error(&error))?
			.map(|value| serde_json::from_str(&value).map_err(StorageError::from))
			.transpose()
	}

	fn set(&self, key: &str, value: &T) -> Result<(), StorageError> {
		let encoded = serde_json::to_string(value)?;
		storage()?
			.set_item(&self.key(key), &encoded)
			.map_err(|error| browser_error(&error))
	}

	fn remove(&self, key: &str) -> Result<(), StorageError> {
		storage()?
			.remove_item(&self.key(key))
			.map_err(|error| browser_error(&error))
	}
}

impl<T> AsyncStorage<T> for WebStorage<T>
where
	T: Serialize + DeserializeOwned + Send + Sync + 'static,
{
	type Error = StorageError;

	fn get_async(&self, key: String) -> StorageFuture<'_, Result<Option<T>, StorageError>> {
		Box::pin(std::future::ready(self.get(&key)))
	}

	fn set_async(&self, key: String, value: T) -> StorageFuture<'_, Result<(), StorageError>> {
		Box::pin(std::future::ready(self.set(&key, &value)))
	}

	fn remove_async(&self, key: String) -> StorageFuture<'_, Result<(), StorageError>> {
		Box::pin(std::future::ready(self.remove(&key)))
	}
}
