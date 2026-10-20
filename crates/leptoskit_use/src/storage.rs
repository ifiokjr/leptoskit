use std::collections::BTreeMap;
use std::convert::Infallible;
use std::sync::Arc;

use parking_lot::Mutex;
use reactive_graph::prelude::*;
use reactive_graph::signal::ReadSignal;
use reactive_graph::signal::RwSignal;

/// Typed, synchronous host storage.
///
/// Implement this for native preferences, browser storage, an SSR request's
/// state, or an embedded key/value store. Encoding belongs to the adapter.
/// Operations must return errors without partially committing writes. Slow or
/// asynchronous stores should be managed outside this synchronous hook.
pub trait Storage<T>: Send + Sync + 'static {
	/// The adapter's concrete error type.
	type Error: Send + Sync + 'static;
	/// Read a value; `None` means the key is absent.
	fn get(&self, key: &str) -> Result<Option<T>, Self::Error>;
	/// Persist a value.
	fn set(&self, key: &str, value: &T) -> Result<(), Self::Error>;
	/// Remove a key. Removing a missing key succeeds.
	fn remove(&self, key: &str) -> Result<(), Self::Error>;
}

/// Shared in-memory storage for tests, headless apps, and request-scoped SSR.
/// Clones share the same store. Create one per request to isolate users.
#[derive(Debug, Clone)]
pub struct MemoryStorage<T> {
	values: Arc<Mutex<BTreeMap<String, T>>>,
}

impl<T> Default for MemoryStorage<T> {
	fn default() -> Self {
		Self {
			values: Arc::default(),
		}
	}
}

impl<T: Clone + Send + Sync + 'static> Storage<T> for MemoryStorage<T> {
	type Error = Infallible;

	fn get(&self, key: &str) -> Result<Option<T>, Self::Error> {
		Ok(self.values.lock().get(key).cloned())
	}

	fn set(&self, key: &str, value: &T) -> Result<(), Self::Error> {
		self.values.lock().insert(key.to_owned(), value.clone());
		Ok(())
	}

	fn remove(&self, key: &str) -> Result<(), Self::Error> {
		self.values.lock().remove(key);
		Ok(())
	}
}

/// A fallible storage action.
pub type StorageAction<E> = Arc<dyn Fn() -> Result<(), E> + Send + Sync>;

/// Reactive storage state with explicit, fallible mutations.
#[derive(Clone)]
pub struct UseStorageReturn<T: Send + Sync + 'static, E> {
	/// Last successfully loaded or persisted value.
	pub value: ReadSignal<T>,
	/// Persist first, then update the signal. Failure preserves the old value.
	pub set_value: Arc<dyn Fn(T) -> Result<(), E> + Send + Sync>,
	/// Remove the key, then reset the signal to its initial fallback.
	pub remove: StorageAction<E>,
	/// Refresh after an external change (for example a browser storage event).
	pub reload: StorageAction<E>,
}

/// Bind a key to host storage. An absent key uses `initial` without writing it.
///
/// Initial read errors are returned to the caller. Subsequent read/write/remove
/// errors also propagate and leave the signal unchanged. No implicit effects,
/// global storage, or cross-request state are created. Use `reload` to
/// integrate platform storage notifications; updates do not automatically sync
/// instances.
pub fn use_storage<T, S>(
	storage: S,
	key: impl Into<String>,
	initial: T,
) -> Result<UseStorageReturn<T, S::Error>, S::Error>
where
	T: Clone + Send + Sync + 'static,
	S: Storage<T>,
{
	let key = key.into();
	let value = RwSignal::new(storage.get(&key)?.unwrap_or_else(|| initial.clone()));
	let storage = Arc::new(storage);
	let writer = Arc::clone(&storage);
	let write_key = key.clone();
	let remover = Arc::clone(&storage);
	let remove_key = key.clone();
	let fallback = initial.clone();
	Ok(UseStorageReturn {
		value: value.read_only(),
		set_value: Arc::new(move |next| {
			writer.set(&write_key, &next)?;
			value.set(next);
			Ok(())
		}),
		remove: Arc::new(move || {
			remover.remove(&remove_key)?;
			value.set(fallback.clone());
			Ok(())
		}),
		reload: Arc::new(move || {
			let next = storage.get(&key)?.unwrap_or_else(|| initial.clone());
			value.set(next);
			Ok(())
		}),
	})
}
