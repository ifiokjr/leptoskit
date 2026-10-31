//! [`AsyncStorage`] over the platform credential store.
//!
//! Values are JSON-encoded secrets bounded by [`crate::MAX_SECRET_BYTES`], so
//! the adapter suits small secrets such as tokens, not bulk settings. Keys that
//! already satisfy the credential name rule keep their readable names; other
//! keys are stored under their SHA-256 hex digests, mirroring how other
//! backends keep arbitrary keys inside their namespace.
//!
//! Browser targets have no credential store, so this module does not compile
//! there.
//!
//! [`AsyncStorage`]: leptoskit_use::AsyncStorage

use std::future::Future;
use std::pin::Pin;

use leptoskit_bridge::Error;
use sha2::Digest;

use crate::Key;

#[cfg(any(
	target_os = "macos",
	target_os = "windows",
	all(target_os = "linux", not(target_env = "ohos"))
))]
type DefaultBackend = crate::SecureStore;
#[cfg(all(target_arch = "wasm32", feature = "guest"))]
type DefaultBackend = crate::guest::SecureStore;

/// One credential operation, polled by the caller's executor.
type BackendFuture<'a, T> = Pin<Box<dyn Future<Output = T> + 'a>>;

/// Failures raised by the secure storage adapter.
#[derive(Debug, thiserror::Error)]
pub enum SecureStorageError {
	/// The credential store rejected the operation, for example because the
	/// encoded secret exceeds [`crate::MAX_SECRET_BYTES`] or the credential
	/// service is locked.
	#[error("secure store failed: {0}")]
	Backend(#[from] Error),
	/// A value could not be encoded as, or decoded from, JSON.
	#[error("secret encoding failed: {0}")]
	Codec(#[from] serde_json::Error),
}

mod private {
	/// Seals [`Backend`](super::Backend) to this crate's implementations.
	pub trait Sealed {}
}

/// The credential operations the platform stores expose.
///
/// Sealed: implemented by the desktop store, the interpreter guest client,
/// shared handles to any backend, and test doubles inside this crate.
pub trait Backend: private::Sealed + Send + Sync + 'static {
	/// Reads a secret; `None` means the key is absent.
	fn read<'a>(&'a self, key: Key<'a>) -> BackendFuture<'a, Result<Option<Vec<u8>>, Error>>;
	/// Stores a bounded secret, replacing any previous value.
	fn write<'a>(&'a self, key: Key<'a>, secret: &'a [u8]) -> BackendFuture<'a, Result<(), Error>>;
	/// Deletes a secret; deleting an absent key succeeds.
	fn erase<'a>(&'a self, key: Key<'a>) -> BackendFuture<'a, Result<(), Error>>;
}

#[cfg(any(
	target_os = "macos",
	target_os = "windows",
	all(target_os = "linux", not(target_env = "ohos"))
))]
mod desktop_backend {
	use super::*;

	impl private::Sealed for crate::SecureStore {}

	impl Backend for crate::SecureStore {
		fn read<'a>(&'a self, key: Key<'a>) -> BackendFuture<'a, Result<Option<Vec<u8>>, Error>> {
			Box::pin(std::future::ready(crate::SecureStore::get(self, key)))
		}

		fn write<'a>(
			&'a self,
			key: Key<'a>,
			secret: &'a [u8],
		) -> BackendFuture<'a, Result<(), Error>> {
			Box::pin(std::future::ready(crate::SecureStore::set(
				self, key, secret,
			)))
		}

		fn erase<'a>(&'a self, key: Key<'a>) -> BackendFuture<'a, Result<(), Error>> {
			Box::pin(std::future::ready(crate::SecureStore::delete(self, key)))
		}
	}
}

#[cfg(all(target_arch = "wasm32", feature = "guest"))]
mod guest_backend {
	use super::*;

	impl private::Sealed for crate::guest::SecureStore {}

	impl Backend for crate::guest::SecureStore {
		fn read<'a>(&'a self, key: Key<'a>) -> BackendFuture<'a, Result<Option<Vec<u8>>, Error>> {
			let client = *self;
			Box::pin(async move { Ok(client.get(key).await?.map(|bytes| bytes.to_vec())) })
		}

		fn write<'a>(
			&'a self,
			key: Key<'a>,
			secret: &'a [u8],
		) -> BackendFuture<'a, Result<(), Error>> {
			let client = *self;
			Box::pin(async move { client.set(key, secret).await })
		}

		fn erase<'a>(&'a self, key: Key<'a>) -> BackendFuture<'a, Result<(), Error>> {
			let client = *self;
			Box::pin(async move { client.delete(key).await })
		}
	}
}

impl<S: Backend> private::Sealed for std::sync::Arc<S> {}

impl<S: Backend> Backend for std::sync::Arc<S> {
	fn read<'a>(&'a self, key: Key<'a>) -> BackendFuture<'a, Result<Option<Vec<u8>>, Error>> {
		(**self).read(key)
	}

	fn write<'a>(&'a self, key: Key<'a>, secret: &'a [u8]) -> BackendFuture<'a, Result<(), Error>> {
		(**self).write(key, secret)
	}

	fn erase<'a>(&'a self, key: Key<'a>) -> BackendFuture<'a, Result<(), Error>> {
		(**self).erase(key)
	}
}

/// [`AsyncStorage`] over a platform credential store.
///
/// The default backend is the desktop store on native targets and the guest
/// client inside interpreter guests. One service namespaces one application's
/// values. Combine with [`leptoskit_use::use_async_storage`] for reactive
/// state.
///
/// [`AsyncStorage`]: leptoskit_use::AsyncStorage
pub struct SecureStorage<T, S: Backend = DefaultBackend> {
	backend: S,
	service: String,
	value: std::marker::PhantomData<fn() -> T>,
}

impl<T, S: Backend> std::fmt::Debug for SecureStorage<T, S> {
	fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		formatter
			.debug_struct("SecureStorage")
			.field("service", &self.service)
			.finish_non_exhaustive()
	}
}

impl<T> SecureStorage<T> {
	/// Opens the desktop credential store for one application and service.
	#[cfg(any(
		target_os = "macos",
		target_os = "windows",
		all(target_os = "linux", not(target_env = "ohos"))
	))]
	pub fn new(application: &str, service: &str) -> Result<Self, SecureStorageError> {
		Ok(Self::with_backend(
			crate::SecureStore::new(application)?,
			service,
		))
	}

	/// Connects to the host's secure-store plugin; the host scopes the
	/// service to the running application.
	#[cfg(all(target_arch = "wasm32", feature = "guest"))]
	pub fn new(service: &str) -> Result<Self, SecureStorageError> {
		Ok(Self::with_backend(
			crate::guest::SecureStore::new()?,
			service,
		))
	}
}

impl<T, S: Backend> SecureStorage<T, S> {
	/// Wraps any [`Backend`], for hosts that construct their own store.
	pub fn with_backend(backend: S, service: &str) -> Self {
		Self {
			backend,
			service: service.to_owned(),
			value: std::marker::PhantomData,
		}
	}

	/// Returns the credential service and name for a storage key: readable
	/// identifiers pass through; everything else is stored under its SHA-256
	/// hex digest.
	fn credential_names<'a>(
		&'a self,
		key: &'a str,
	) -> (std::borrow::Cow<'a, str>, std::borrow::Cow<'a, str>) {
		let name = if is_credential_name(key) {
			std::borrow::Cow::Borrowed(key)
		} else {
			std::borrow::Cow::Owned(format!("{:x}", sha2::Sha256::digest(key.as_bytes())))
		};
		(std::borrow::Cow::Borrowed(self.service.as_str()), name)
	}
}

/// Whether a storage key can serve verbatim as a credential name.
fn is_credential_name(key: &str) -> bool {
	!key.is_empty()
		&& key.len() <= crate::MAX_NAME_BYTES
		&& key
			.bytes()
			.all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

impl<T, S> leptoskit_use::AsyncStorage<T> for SecureStorage<T, S>
where
	T: serde::Serialize + serde::de::DeserializeOwned + 'static,
	S: Backend,
{
	type Error = SecureStorageError;

	fn get_async(
		&self,
		key: String,
	) -> leptoskit_use::StorageFuture<'_, Result<Option<T>, Self::Error>> {
		Box::pin(async move {
			let (service, name) = self.credential_names(&key);
			let credential = Key::new(&service, &name).map_err(SecureStorageError::from)?;
			let secret = self
				.backend
				.read(credential)
				.await
				.map_err(SecureStorageError::from)?;
			secret
				.map(|bytes| serde_json::from_slice(&bytes))
				.transpose()
				.map_err(SecureStorageError::from)
		})
	}

	fn set_async(
		&self,
		key: String,
		value: T,
	) -> leptoskit_use::StorageFuture<'_, Result<(), Self::Error>> {
		Box::pin(async move {
			let secret = serde_json::to_vec(&value).map_err(SecureStorageError::from)?;
			let (service, name) = self.credential_names(&key);
			let credential = Key::new(&service, &name).map_err(SecureStorageError::from)?;
			self.backend
				.write(credential, &secret)
				.await
				.map_err(SecureStorageError::from)
		})
	}

	fn remove_async(
		&self,
		key: String,
	) -> leptoskit_use::StorageFuture<'_, Result<(), Self::Error>> {
		Box::pin(async move {
			let (service, name) = self.credential_names(&key);
			let credential = Key::new(&service, &name).map_err(SecureStorageError::from)?;
			self.backend
				.erase(credential)
				.await
				.map_err(SecureStorageError::from)
		})
	}
}

#[cfg(all(
	any(
		target_os = "macos",
		target_os = "windows",
		all(target_os = "linux", not(target_env = "ohos"))
	),
	test
))]
mod tests {
	use std::collections::BTreeMap;
	use std::sync::Arc;
	use std::sync::atomic::AtomicBool;

	use futures::executor::block_on;
	use leptoskit_bridge::Error;
	use leptoskit_use::AsyncStorage;
	use leptoskit_use::use_async_storage;
	use reactive_graph::owner::Owner;
	use reactive_graph::prelude::Get;
	use serde::Deserialize;
	use serde::Serialize;

	use super::Backend;
	use super::Key;
	use super::SecureStorage;
	use super::SecureStorageError;
	use crate::MAX_SECRET_BYTES;

	#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
	struct Session {
		token: String,
	}

	/// In-memory backend mirroring the platform bounds.
	struct Fake {
		entries: std::sync::Mutex<BTreeMap<(String, String), Vec<u8>>>,
		failing: AtomicBool,
	}

	impl Fake {
		fn new() -> Self {
			Self {
				entries: std::sync::Mutex::new(BTreeMap::new()),
				failing: AtomicBool::new(false),
			}
		}

		fn set_failing(&self) {
			self.failing
				.store(true, std::sync::atomic::Ordering::Relaxed);
		}
	}

	impl super::private::Sealed for Fake {}

	impl Backend for Fake {
		fn read<'a>(
			&'a self,
			key: Key<'a>,
		) -> super::BackendFuture<'a, Result<Option<Vec<u8>>, Error>> {
			let failing = self.failing.load(std::sync::atomic::Ordering::Relaxed);
			Box::pin(std::future::ready(if failing {
				Err(Error::PermissionDenied)
			} else {
				Ok(self
					.entries
					.lock()
					.unwrap()
					.get(&(key.service().to_owned(), key.key().to_owned()))
					.cloned())
			}))
		}

		fn write<'a>(
			&'a self,
			key: Key<'a>,
			secret: &'a [u8],
		) -> super::BackendFuture<'a, Result<(), Error>> {
			let failing = self.failing.load(std::sync::atomic::Ordering::Relaxed);
			Box::pin(std::future::ready(if failing {
				Err(Error::PermissionDenied)
			} else if secret.len() > MAX_SECRET_BYTES {
				Err(Error::TooLarge)
			} else {
				self.entries.lock().unwrap().insert(
					(key.service().to_owned(), key.key().to_owned()),
					secret.to_vec(),
				);
				Ok(())
			}))
		}

		fn erase<'a>(&'a self, key: Key<'a>) -> super::BackendFuture<'a, Result<(), Error>> {
			let failing = self.failing.load(std::sync::atomic::Ordering::Relaxed);
			Box::pin(std::future::ready(if failing {
				Err(Error::PermissionDenied)
			} else {
				self.entries
					.lock()
					.unwrap()
					.remove(&(key.service().to_owned(), key.key().to_owned()));
				Ok(())
			}))
		}
	}

	fn storage(backend: &Arc<Fake>) -> SecureStorage<Session, Arc<Fake>> {
		SecureStorage::with_backend(Arc::clone(backend), "login")
	}

	#[test]
	fn adapter_round_trips_and_resets_reactive_state() {
		let owner = Owner::new();
		let backend = Arc::new(Fake::new());
		let hook = block_on(owner.with(|| {
			use_async_storage(
				storage(&backend),
				"session",
				Session {
					token: "guest".into(),
				},
			)
		}))
		.unwrap();
		assert_eq!(
			hook.value.get(),
			Session {
				token: "guest".into()
			}
		);
		block_on((hook.set_value)(Session {
			token: "issued".into(),
		}))
		.unwrap();
		assert_eq!(
			hook.value.get(),
			Session {
				token: "issued".into()
			}
		);
		block_on((hook.reload)()).unwrap();
		assert_eq!(
			hook.value.get(),
			Session {
				token: "issued".into()
			}
		);
		block_on((hook.remove)()).unwrap();
		assert_eq!(
			hook.value.get(),
			Session {
				token: "guest".into()
			}
		);
		assert!(backend.entries.lock().unwrap().is_empty());
		owner.cleanup();
	}

	#[test]
	fn readable_keys_pass_through_and_other_keys_are_hashed() {
		let backend = Arc::new(Fake::new());
		let storage = storage(&backend);
		block_on(storage.set_async(
			"token".into(),
			Session {
				token: "one".into(),
			},
		))
		.unwrap();
		block_on(storage.set_async(
			"auth/token".into(),
			Session {
				token: "two".into(),
			},
		))
		.unwrap();
		assert!(
			backend
				.entries
				.lock()
				.unwrap()
				.contains_key(&("login".to_owned(), "token".to_owned()))
		);
		assert!(
			backend
				.entries
				.lock()
				.unwrap()
				.keys()
				.any(|(service, key)| service == "login" && key.len() == 64 && key != "auth/token")
		);
		assert_eq!(
			block_on(storage.get_async("auth/token".into())).unwrap(),
			Some(Session {
				token: "two".into()
			})
		);
	}

	#[test]
	fn oversized_secrets_and_locked_services_surface_backend_errors() {
		let owner = Owner::new();
		let backend = Arc::new(Fake::new());
		let hook = block_on(owner.with(|| {
			use_async_storage(
				storage(&backend),
				"session",
				Session {
					token: "guest".into(),
				},
			)
		}))
		.unwrap();
		block_on((hook.set_value)(Session {
			token: "issued".into(),
		}))
		.unwrap();
		let oversized = Session {
			token: "x".repeat(MAX_SECRET_BYTES + 1),
		};
		let error = block_on((hook.set_value)(oversized)).unwrap_err();
		assert!(matches!(
			error,
			leptoskit_use::AsyncStorageError::Backend(SecureStorageError::Backend(Error::TooLarge))
		));
		assert_eq!(
			hook.value.get(),
			Session {
				token: "issued".into()
			}
		);

		backend.set_failing();
		assert!(matches!(
			block_on((hook.reload)()).unwrap_err(),
			leptoskit_use::AsyncStorageError::Backend(SecureStorageError::Backend(_))
		));
		assert_eq!(
			hook.value.get(),
			Session {
				token: "issued".into()
			}
		);
		owner.cleanup();
	}

	#[test]
	fn undecodable_secrets_surface_codec_errors() {
		let backend = Arc::new(Fake::new());
		backend.entries.lock().unwrap().insert(
			("login".to_owned(), "session".to_owned()),
			b"not json".to_vec(),
		);
		let storage = storage(&backend);
		assert!(matches!(
			block_on(storage.get_async("session".into())).unwrap_err(),
			SecureStorageError::Codec(_)
		));
	}

	#[test]
	fn desktop_backend_delegates_to_the_credential_store() {
		keyring::set_default_credential_builder(keyring::mock::default_credential_builder());
		let store = crate::SecureStore::new("dev.leptoskit.test").unwrap();
		let key = Key::new("login", "token").unwrap();
		assert_eq!(
			block_on(Backend::write(&store, key, b"secret")).unwrap(),
			()
		);
		// The mock backend keeps every entry independent, so a fresh lookup
		// starts absent; delegation, not persistence, is under test.
		assert_eq!(block_on(Backend::read(&store, key)).unwrap(), None);
		assert_eq!(block_on(Backend::erase(&store, key)).unwrap(), ());
	}
}
