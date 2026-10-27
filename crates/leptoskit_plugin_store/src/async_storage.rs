//! [`AsyncStorage`] adapters over the store plugin's settings backends.
//!
//! One adapter type covers the synchronous native and browser stores; the
//! interpreter guest client implements the same contract directly. `set` and
//! `remove` commit the whole document before publishing, matching the
//! [`AsyncStorage`] requirement that failed writes preserve the previously
//! committed value.
//!
//! [`AsyncStorage`]: leptoskit_use::AsyncStorage

use leptoskit_bridge::Error;

/// Failures raised by the store adapters.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
	/// The store backend rejected the operation, for example because a key is
	/// empty or contains control characters, a value exceeds the size budget,
	/// or browser storage is unavailable.
	#[error("store backend failed: {0}")]
	Backend(#[from] Error),
	/// A value could not be encoded as, or decoded from, JSON.
	#[error("store value encoding failed: {0}")]
	Codec(#[from] serde_json::Error),
}

/// Synchronous backends that share one adapter.
#[cfg(any(
	not(target_arch = "wasm32"),
	all(target_arch = "wasm32", target_os = "unknown", not(feature = "guest"))
))]
mod sync {
	use std::marker::PhantomData;

	use leptoskit_bridge::Error;
	use leptoskit_use::AsyncStorage;
	use leptoskit_use::StorageFuture;
	use serde::Serialize;
	use serde::de::DeserializeOwned;

	use super::StoreError;
	use crate::Change;
	use crate::Save;
	use crate::Value;

	/// The synchronous store compiled into this target.
	#[cfg(not(target_arch = "wasm32"))]
	type SyncStore = crate::Store;
	#[cfg(all(target_arch = "wasm32", target_os = "unknown", not(feature = "guest")))]
	type SyncStore = crate::web::Store;

	/// Reads one entry, cloning the stored JSON.
	fn read(store: &parking_lot::Mutex<SyncStore>, name: &str) -> Result<Option<Value>, Error> {
		let mut guard = store.lock();
		let value = guard.get(name)?;
		Ok(value.cloned())
	}

	/// Applies one committed batch.
	fn commit(store: &parking_lot::Mutex<SyncStore>, changes: &[Change]) -> Result<(), Error> {
		store.lock().apply(changes, Save::Commit)
	}

	/// [`AsyncStorage`] over the target's synchronous native or browser store.
	///
	/// Operations lock the store briefly and complete immediately. Keys keep
	/// their verbatim names, so they must be nonempty, free of control
	/// characters, and at most [`crate::MAX_KEY_BYTES`] UTF-8 bytes. Combine
	/// with [`leptoskit_use::use_async_storage`] for reactive state.
	///
	/// [`AsyncStorage`]: leptoskit_use::AsyncStorage
	pub struct StoreStorage<T> {
		store: parking_lot::Mutex<SyncStore>,
		value: PhantomData<fn() -> T>,
	}

	impl<T> std::fmt::Debug for StoreStorage<T> {
		fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
			formatter
				.debug_struct("StoreStorage")
				.finish_non_exhaustive()
		}
	}

	impl<T> StoreStorage<T> {
		/// Wraps a store opened by the host: [`crate::Store::new`] on native or
		/// [`crate::web::Store::open`] in the browser.
		pub fn new(store: SyncStore) -> Self {
			Self {
				store: parking_lot::Mutex::new(store),
				value: PhantomData,
			}
		}
	}

	impl<T> AsyncStorage<T> for StoreStorage<T>
	where
		T: Serialize + DeserializeOwned + 'static,
	{
		type Error = StoreError;

		fn get_async(&self, key: String) -> StorageFuture<'_, Result<Option<T>, Self::Error>> {
			Box::pin(std::future::ready((|| {
				let value = read(&self.store, &key).map_err(StoreError::from)?;
				value
					.map(serde_json::from_value)
					.transpose()
					.map_err(StoreError::from)
			})()))
		}

		fn set_async(&self, key: String, value: T) -> StorageFuture<'_, Result<(), Self::Error>> {
			Box::pin(std::future::ready(
				serde_json::to_value(&value)
					.map_err(StoreError::from)
					.and_then(|value| {
						commit(&self.store, &[Change::Set(key, value)]).map_err(StoreError::from)
					}),
			))
		}

		fn remove_async(&self, key: String) -> StorageFuture<'_, Result<(), Self::Error>> {
			Box::pin(std::future::ready(
				commit(&self.store, &[Change::Remove(key)]).map_err(StoreError::from),
			))
		}
	}
}

#[cfg(any(
	not(target_arch = "wasm32"),
	all(target_arch = "wasm32", target_os = "unknown", not(feature = "guest"))
))]
pub use sync::StoreStorage;

/// The interpreter guest client already exposes asynchronous operations, so it
/// implements the contract without a wrapper.
#[cfg(all(target_arch = "wasm32", feature = "guest"))]
mod guest {
	use leptoskit_use::AsyncStorage;
	use leptoskit_use::StorageFuture;
	use serde::Serialize;
	use serde::de::DeserializeOwned;

	use super::StoreError;
	use crate::Change;
	use crate::Save;

	impl<T> AsyncStorage<T> for crate::guest::Store
	where
		T: Serialize + DeserializeOwned + 'static,
	{
		type Error = StoreError;

		fn get_async(&self, key: String) -> StorageFuture<'_, Result<Option<T>, Self::Error>> {
			let client = *self;
			Box::pin(async move {
				let value = client.get(&key).await.map_err(StoreError::from)?;
				value
					.map(serde_json::from_value)
					.transpose()
					.map_err(StoreError::from)
			})
		}

		fn set_async(&self, key: String, value: T) -> StorageFuture<'_, Result<(), Self::Error>> {
			let client = *self;
			Box::pin(async move {
				let value = serde_json::to_value(&value).map_err(StoreError::from)?;
				client
					.apply(&[Change::Set(key, value)], Save::Commit)
					.await
					.map_err(StoreError::from)
			})
		}

		fn remove_async(&self, key: String) -> StorageFuture<'_, Result<(), Self::Error>> {
			let client = *self;
			Box::pin(async move {
				client
					.apply(&[Change::Remove(key)], Save::Commit)
					.await
					.map_err(StoreError::from)
			})
		}
	}
}

#[cfg(all(not(target_arch = "wasm32"), test))]
mod tests {
	use futures::executor::block_on;
	use leptoskit_use::AsyncStorage;
	use leptoskit_use::use_async_storage;
	use reactive_graph::owner::Owner;
	use reactive_graph::prelude::Get;

	use super::StoreError;
	use super::StoreStorage;
	use crate::Value;

	#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
	struct Settings {
		name: String,
	}

	fn directory() -> std::path::PathBuf {
		tempfile::tempdir().unwrap().keep()
	}

	#[test]
	fn adapter_round_trips_values_through_the_native_store() {
		let owner = Owner::new();
		let storage = StoreStorage::<Settings>::new(crate::Store::new(directory()));
		let hook = block_on(
			owner.with(|| use_async_storage(storage, "profile", Settings { name: "Ada".into() })),
		)
		.unwrap();
		assert_eq!(hook.value.get(), Settings { name: "Ada".into() });
		block_on((hook.set_value)(Settings {
			name: "Grace".into(),
		}))
		.unwrap();
		assert_eq!(
			hook.value.get(),
			Settings {
				name: "Grace".into()
			}
		);
		block_on((hook.remove)()).unwrap();
		assert_eq!(hook.value.get(), Settings { name: "Ada".into() });
		owner.cleanup();
	}

	#[test]
	fn reopened_storage_observes_committed_values() {
		let path = directory();
		let owner = Owner::new();
		let first = StoreStorage::<Settings>::new(crate::Store::new(&path));
		let hook = block_on(
			owner.with(|| use_async_storage(first, "profile", Settings { name: "Ada".into() })),
		)
		.unwrap();
		block_on((hook.set_value)(Settings {
			name: "Grace".into(),
		}))
		.unwrap();
		owner.cleanup();

		let reopened = StoreStorage::<Settings>::new(crate::Store::new(&path));
		assert_eq!(
			block_on(reopened.get_async("profile".into())).unwrap(),
			Some(Settings {
				name: "Grace".into()
			})
		);
		block_on(reopened.remove_async("profile".into())).unwrap();
		assert_eq!(
			block_on(reopened.get_async("profile".into())).unwrap(),
			None
		);
	}

	#[test]
	fn failed_writes_preserve_the_last_committed_value() {
		let owner = Owner::new();
		let storage = StoreStorage::<Settings>::new(crate::Store::new(directory()));
		let hook = block_on(
			owner.with(|| use_async_storage(storage, "profile", Settings { name: "Ada".into() })),
		)
		.unwrap();
		block_on((hook.set_value)(Settings {
			name: "Grace".into(),
		}))
		.unwrap();
		let oversized = Settings {
			name: "x".repeat(crate::MAX_VALUE_BYTES + 1),
		};
		assert!(block_on((hook.set_value)(oversized)).is_err());
		assert_eq!(
			hook.value.get(),
			Settings {
				name: "Grace".into()
			}
		);
		owner.cleanup();
	}

	#[test]
	fn invalid_keys_and_undecodable_values_surface_errors() {
		let path = directory();
		let mut store = crate::Store::new(&path);
		store.set("profile", Value::Bool(true)).unwrap();
		let storage = StoreStorage::<Settings>::new(store);
		assert!(matches!(
			block_on(storage.get_async("profile".into())).unwrap_err(),
			StoreError::Codec(_)
		));
		let storage = StoreStorage::<u32>::new(crate::Store::new(path));
		assert!(matches!(
			block_on(storage.set_async(String::new(), 7_u32)).unwrap_err(),
			StoreError::Backend(_)
		));
	}
}
