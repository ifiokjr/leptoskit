//! Small binary secrets in the OS credential store, scoped to an application.
//!
//! Native mobile handlers ship with this crate. Desktop applications construct
//! `SecureStore` with a stable application identifier. Browser storage is
//! deliberately unavailable: origin storage does not provide a native keychain.
//! Values are limited to 2048 bytes across platforms. No biometric prompt,
//! shared access group, cloud synchronization, or secret enumeration is
//! exposed.

use leptoskit_bridge::Bytes;
use leptoskit_bridge::Error;
use leptoskit_bridge::MethodId;
use leptoskit_bridge::codec::Decoder;
use leptoskit_bridge::codec::Encoder;

/// Stable native registration name.
pub const NAME: &str = "secure-store";
/// Service and key -> presence u32 followed by raw secret bytes when present.
pub const GET: MethodId = MethodId(0);
/// Service, key and length-prefixed binary secret -> empty response.
pub const SET: MethodId = MethodId(1);
/// Service and key -> empty response. Missing entries are successful deletions.
pub const DELETE: MethodId = MethodId(2);
/// Supported operations.
pub const METHODS: &[MethodId] = &[GET, SET, DELETE];
/// Maximum encoded bytes in a secret, including an empty secret.
pub const MAX_SECRET_BYTES: usize = 2048;
/// Maximum ASCII bytes in a service or key.
pub const MAX_NAME_BYTES: usize = 128;

mod archive;
pub use archive::Quota;

/// Validated service/key pair. Names permit ASCII letters, digits, '.', '_' and
/// '-'; they cannot be empty. A service is an application-local namespace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Key<'a> {
	service: &'a str,
	key: &'a str,
}

impl<'a> Key<'a> {
	/// Validates a service and key without allocating.
	pub fn new(service: &'a str, key: &'a str) -> Result<Self, Error> {
		validate_name(service, MAX_NAME_BYTES)?;
		validate_name(key, MAX_NAME_BYTES)?;
		Ok(Self { service, key })
	}

	/// Application-local service name.
	pub fn service(self) -> &'a str {
		self.service
	}

	/// Secret's key within its service.
	pub fn key(self) -> &'a str {
		self.key
	}

	/// Encodes a get/delete request or a set request with `Some(secret)`.
	pub fn encode(self, secret: Option<&[u8]>) -> Result<Bytes, Error> {
		if secret.is_some_and(|value| value.len() > MAX_SECRET_BYTES) {
			return Err(Error::TooLarge);
		}

		let mut output = Encoder::with_capacity(
			12 + self.service.len() + self.key.len() + secret.map_or(0, <[u8]>::len),
		);
		output.bytes(self.service.as_bytes())?;
		output.bytes(self.key.as_bytes())?;

		if let Some(secret) = secret {
			output.bytes(secret)?;
		}

		Ok(output.finish())
	}
}

fn validate_name(name: &str, limit: usize) -> Result<(), Error> {
	if name.is_empty()
		|| name.len() > limit
		|| !name
			.bytes()
			.all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
	{
		return Err(Error::InvalidArgument);
	}
	Ok(())
}

/// Validated request that borrows its payload; does not copy the secret.
/// Intentionally lacks `Debug` to avoid formatting secrets into logs.
pub struct Request<'a> {
	/// Target entry.
	pub key: Key<'a>,
	/// Present only for `SET` (including an empty value).
	pub secret: Option<&'a [u8]>,
}

impl<'a> Request<'a> {
	/// Rejects unknown methods, malformed framing, oversized values and
	/// trailers.
	pub fn decode(method: MethodId, payload: &'a [u8]) -> Result<Self, Error> {
		if !METHODS.contains(&method) {
			return Err(Error::Unavailable);
		}

		let mut input = Decoder::new(payload);
		let key = Key::new(input.string()?, input.string()?)?;
		let secret = if method == SET {
			Some(input.bytes()?)
		} else {
			None
		};

		if secret.is_some_and(|value| value.len() > MAX_SECRET_BYTES) {
			return Err(Error::TooLarge);
		}

		input.finish()?;
		Ok(Self { key, secret })
	}
}

/// Decodes a get response, preserving the difference between absent and empty.
pub fn decode_secret(response: &[u8]) -> Result<Option<&[u8]>, Error> {
	let (flag, secret) = response.split_at_checked(4).ok_or(Error::Native)?;

	match flag {
		[0, 0, 0, 0] if secret.is_empty() => Ok(None),

		[1, 0, 0, 0] if secret.len() <= MAX_SECRET_BYTES => Ok(Some(secret)),
		_ => Err(Error::Native),
	}
}

#[cfg(any(
	target_os = "macos",
	target_os = "windows",
	all(target_os = "linux", not(target_env = "ohos"))
))]
mod desktop {
	use std::sync::Mutex;

	use leptoskit_bridge::Plugin;

	use super::*;
	use crate::archive::Archive;

	/// OS credential store scoped to a stable application ID. Desktop keychains
	/// belong to the logged-in user; the namespace is not an authorization
	/// boundary. Calls can block on the OS credential service; invoke off
	/// the UI thread.
	pub struct SecureStore {
		application: String,
		quota: Quota,
		#[cfg(test)]
		test_store: Option<std::sync::Arc<Mutex<tests::MemoryStore>>>,
	}

	// Serializes read/modify/write across every store instance in this host.
	static STORE_LOCK: Mutex<()> = Mutex::new(());

	impl SecureStore {
		/// Validates an application ID (1–255 ASCII name bytes). No secret is
		/// read.
		pub fn new(application: &str) -> Result<Self, Error> {
			Self::with_quota(application, Quota::default())
		}

		/// Selects aggregate limits for this app's single encrypted credential.
		pub fn with_quota(application: &str, quota: Quota) -> Result<Self, Error> {
			validate_name(application, 255)?;
			quota.validate()?;
			Ok(Self {
				application: application.to_owned(),
				quota,
				#[cfg(test)]
				test_store: None,
			})
		}

		fn lock(&self) -> Result<std::fs::File, Error> {
			#[cfg(not(test))]
			let directory = dirs::data_local_dir().ok_or(Error::Unavailable)?;
			#[cfg(test)]
			let directory = std::env::temp_dir().join("leptoskit-secure-store-tests");
			let directory = directory
				.join("leptoskit-secure-store-locks")
				.join(&self.application);
			let mut builder = std::fs::DirBuilder::new();
			builder.recursive(true);
			#[cfg(unix)]
			{
				use std::os::unix::fs::DirBuilderExt as _;
				builder.mode(0o700);
			}
			builder.create(&directory).map_err(|_| Error::Native)?;
			let mut options = std::fs::OpenOptions::new();
			options.read(true).write(true).create(true).truncate(false);
			#[cfg(unix)]
			{
				use std::os::unix::fs::OpenOptionsExt as _;
				options
					.mode(0o600)
					.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
			}
			let file = options
				.open(directory.join(".lock"))
				.map_err(|_| Error::Native)?;

			if !file.metadata().map_err(|_| Error::Native)?.is_file() {
				return Err(Error::PermissionDenied);
			}

			fs4::FileExt::lock(&file).map_err(|_| Error::Native)?;
			Ok(file)
		}

		fn entry(&self, key: Key<'_>) -> Result<keyring::Entry, Error> {
			let service = format!(
				"leptoskit.secure-store:{}:{}",
				self.application,
				key.service()
			);

			#[cfg(test)]
			if let Some(entry) = self.mock_entry(&service, key.key()) {
				return Ok(entry);
			}

			keyring::Entry::new(&service, key.key()).map_err(|error| map_error(&error))
		}

		#[cfg(test)]
		fn mock_entry(&self, service: &str, key: &str) -> Option<keyring::Entry> {
			self.test_store.as_ref().map(|store| {
				keyring::Entry::new_with_credential(Box::new(tests::MemoryCredential {
					store: std::sync::Arc::clone(store),
					identity: (service.to_owned(), key.to_owned()),
				}))
			})
		}

		fn archive_entry(&self) -> Result<keyring::Entry, Error> {
			let service = format!("leptoskit.secure-store-v2:{}", self.application);

			#[cfg(test)]
			if let Some(entry) = self.mock_entry(&service, "entries") {
				return Ok(entry);
			}

			keyring::Entry::new(&service, "entries").map_err(|error| map_error(&error))
		}

		fn archive(&self) -> Result<Archive, Error> {
			match self.archive_entry()?.get_secret() {
				Ok(bytes) => Archive::decode(&bytes),
				Err(keyring::Error::NoEntry) => Ok(Archive::default()),
				Err(error) => Err(map_error(&error)),
			}
		}

		fn delete_legacy(&self, key: Key<'_>) -> Result<(), Error> {
			match self.entry(key)?.delete_credential() {
				Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
				Err(error) => Err(map_error(&error)),
			}
		}

		/// Reads binary data. A missing entry returns `None`; a
		/// locked/unavailable credential service returns an error,
		/// never a fabricated missing value.
		pub fn get(&self, key: Key<'_>) -> Result<Option<Vec<u8>>, Error> {
			let _lock = STORE_LOCK.lock().map_err(|_| Error::Native)?;
			let _file = self.lock()?;

			if let Some(value) = self.archive()?.get(key) {
				return Ok(Some(value.to_vec()));
			}

			match self.entry(key)?.get_secret() {
				Ok(value) if value.len() <= MAX_SECRET_BYTES => Ok(Some(value)),
				Ok(_) => Err(Error::TooLarge),
				Err(keyring::Error::NoEntry) => Ok(None),
				Err(error) => Err(map_error(&error)),
			}
		}

		/// Replaces one secret. No plaintext file fallback is used.
		pub fn set(&self, key: Key<'_>, secret: &[u8]) -> Result<(), Error> {
			if secret.len() > MAX_SECRET_BYTES {
				return Err(Error::TooLarge);
			}

			let _lock = STORE_LOCK.lock().map_err(|_| Error::Native)?;
			let _file = self.lock()?;
			let mut archive = self.archive()?;
			archive.set(key, secret);
			let bytes = archive.encode(self.quota)?;
			self.archive_entry()?
				.set_secret(&bytes)
				.map_err(|error| map_error(&error))?;

			self.delete_legacy(key)
		}

		/// Removes an entry; deleting an absent entry succeeds.
		pub fn delete(&self, key: Key<'_>) -> Result<(), Error> {
			let _lock = STORE_LOCK.lock().map_err(|_| Error::Native)?;
			let _file = self.lock()?;
			let mut archive = self.archive()?;
			// Delete the old representation first so it cannot reappear after
			// removal.
			self.delete_legacy(key)?;
			archive.remove(key);
			// A lowered policy must still allow cleanup of an existing store.
			let bytes = archive.encode(Quota::default())?;
			let entry = self.archive_entry()?;

			if bytes.len() == 4 {
				return match entry.delete_credential() {
					Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
					Err(error) => Err(map_error(&error)),
				};
			}

			entry.set_secret(&bytes).map_err(|error| map_error(&error))
		}
	}

	fn map_error(error: &keyring::Error) -> Error {
		match error {
			keyring::Error::NoStorageAccess(_) => Error::PermissionDenied,
			keyring::Error::TooLong(..) => Error::TooLarge,
			keyring::Error::Invalid(..) => Error::InvalidArgument,
			_ => Error::Native,
		}
	}

	impl Plugin for SecureStore {
		fn name(&self) -> &'static str {
			NAME
		}

		fn methods(&self) -> &'static [MethodId] {
			METHODS
		}

		fn invoke(&mut self, method: MethodId, payload: Bytes) -> Result<Bytes, Error> {
			let request = Request::decode(method, &payload)?;

			match method {
				GET => {
					let value = self.get(request.key)?;
					let mut response = Vec::with_capacity(4 + value.as_ref().map_or(0, Vec::len));
					response.extend_from_slice(&u32::from(value.is_some()).to_le_bytes());
					if let Some(value) = value {
						response.extend_from_slice(&value);
					}

					Ok(response.into())
				}
				SET => {
					self.set(request.key, request.secret.ok_or(Error::InvalidArgument)?)?;
					Ok(Bytes::new())
				}
				DELETE => {
					self.delete(request.key)?;
					Ok(Bytes::new())
				}
				_ => Err(Error::Unavailable),
			}
		}
	}

	#[cfg(test)]
	mod tests {
		use std::sync::Arc;

		use super::*;

		#[derive(Default)]
		pub(super) struct MemoryStore {
			entries: std::collections::BTreeMap<(String, String), Vec<u8>>,
			fail_write: bool,
		}

		pub(super) struct MemoryCredential {
			pub store: Arc<Mutex<MemoryStore>>,
			pub identity: (String, String),
		}

		impl keyring::credential::CredentialApi for MemoryCredential {
			fn set_secret(&self, value: &[u8]) -> keyring::Result<()> {
				let mut store = self.store.lock().unwrap();

				if store.fail_write {
					return Err(keyring::Error::PlatformFailure(Box::new(
						std::io::Error::other("injected write failure"),
					)));
				}

				store.entries.insert(self.identity.clone(), value.to_vec());
				Ok(())
			}

			fn get_secret(&self) -> keyring::Result<Vec<u8>> {
				self.store
					.lock()
					.unwrap()
					.entries
					.get(&self.identity)
					.cloned()
					.ok_or(keyring::Error::NoEntry)
			}

			fn delete_credential(&self) -> keyring::Result<()> {
				self.store
					.lock()
					.unwrap()
					.entries
					.remove(&self.identity)
					.map(|_| ())
					.ok_or(keyring::Error::NoEntry)
			}

			fn as_any(&self) -> &dyn std::any::Any {
				self
			}
		}

		fn memory_store() -> SecureStore {
			let mut store = SecureStore::with_quota(
				"dev.leptoskit.quota-test",
				Quota {
					max_bytes: 128,
					max_entries: 2,
				},
			)
			.unwrap();
			store.test_store = Some(Arc::default());
			store
		}

		#[test]
		fn quota_failure_and_native_failure_preserve_persistent_values() {
			let store = memory_store();
			let a = Key::new("login", "a").unwrap();
			let b = Key::new("login", "b").unwrap();
			let c = Key::new("login", "c").unwrap();
			store.set(a, b"old").unwrap();
			store.set(b, b"").unwrap();
			assert_eq!(store.set(c, b""), Err(Error::TooLarge));
			assert_eq!(store.set(a, &[0; 128]), Err(Error::TooLarge));

			assert_eq!(store.get(a).unwrap().as_deref(), Some(b"old".as_slice()));
			let backend = Arc::clone(store.test_store.as_ref().unwrap());
			backend.lock().unwrap().fail_write = true;
			assert_eq!(store.set(a, b"replacement"), Err(Error::Native));
			backend.lock().unwrap().fail_write = false;
			drop(store);
			let mut reopened = memory_store();
			reopened.test_store = Some(backend);
			assert_eq!(reopened.get(a).unwrap().as_deref(), Some(b"old".as_slice()));
			reopened.delete(a).unwrap();
			reopened.set(c, b"new").unwrap();
			assert_eq!(reopened.get(a).unwrap(), None);
			assert_eq!(reopened.get(b).unwrap(), Some(vec![]));
			assert_eq!(reopened.get(c).unwrap().as_deref(), Some(b"new".as_slice()));
		}

		#[test]
		fn legacy_values_migrate_on_write_and_corruption_never_becomes_absence() {
			let store = memory_store();
			let key = Key::new("login", "token").unwrap();
			store.entry(key).unwrap().set_secret(b"legacy").unwrap();
			assert_eq!(
				store.get(key).unwrap().as_deref(),
				Some(b"legacy".as_slice())
			);
			store.set(key, b"current").unwrap();
			assert!(matches!(
				store.entry(key).unwrap().get_secret(),
				Err(keyring::Error::NoEntry)
			));
			store
				.archive_entry()
				.unwrap()
				.set_secret(b"corrupt")
				.unwrap();
			assert!(store.get(key).is_err());

			assert!(store.set(key, b"overwrite").is_err());
			assert_eq!(
				store.archive_entry().unwrap().get_secret().unwrap(),
				b"corrupt"
			);
		}

		#[test]
		fn concurrent_fresh_keys_cannot_bypass_the_entry_budget() {
			let store = Arc::new(memory_store());
			let workers: Vec<_> = (0..8)
				.map(|index| {
					let store = Arc::clone(&store);
					std::thread::spawn(move || {
						store.set(Key::new("login", &index.to_string()).unwrap(), b"")
					})
				})
				.collect();
			assert_eq!(
				workers
					.into_iter()
					.map(|worker| worker.join().unwrap())
					.inspect(|result| assert!(matches!(result, Ok(()) | Err(Error::TooLarge))))
					.flatten()
					.count(),
				2
			);
		}

		/// keyring's mock backend never touches the user's credential store.
		/// Its entries are independent, so every lookup starts absent.
		fn store() -> SecureStore {
			keyring::set_default_credential_builder(keyring::mock::default_credential_builder());
			SecureStore::new("dev.leptoskit.test").unwrap()
		}

		#[test]
		fn application_ids_are_validated_before_any_lookup() {
			assert_eq!(SecureStore::new("").err(), Some(Error::InvalidArgument));
			assert_eq!(
				SecureStore::new("bad id").err(),
				Some(Error::InvalidArgument)
			);
		}

		#[test]
		fn dispatch_encodes_absent_values_and_accepts_writes_and_deletes() {
			let mut store = store();
			assert_eq!(store.name(), NAME);
			assert_eq!(store.methods(), METHODS);
			let key = Key::new("login", "token").unwrap();
			let set = store.invoke(SET, key.encode(Some(b"secret")).unwrap());
			assert_eq!(set, Ok(Bytes::new()));
			let absent = store.invoke(GET, key.encode(None).unwrap()).unwrap();
			assert_eq!(decode_secret(&absent), Ok(None));

			assert_eq!(
				store.invoke(DELETE, key.encode(None).unwrap()),
				Ok(Bytes::new())
			);
			assert_eq!(
				store.set(key, &vec![0; MAX_SECRET_BYTES + 1]),
				Err(Error::TooLarge)
			);
			assert_eq!(
				store.invoke(MethodId(9), Bytes::new()),
				Err(Error::Unavailable)
			);
		}

		#[test]
		fn credential_failures_map_to_stable_bridge_errors() {
			let denied = keyring::Error::NoStorageAccess(Box::new(std::io::Error::other("locked")));
			assert_eq!(map_error(&denied), Error::PermissionDenied);
			assert_eq!(
				map_error(&keyring::Error::TooLong("secret".into(), 1)),
				Error::TooLarge
			);
			assert_eq!(
				map_error(&keyring::Error::Invalid("user".into(), "bad".into())),
				Error::InvalidArgument
			);
			assert_eq!(map_error(&keyring::Error::NoEntry), Error::Native);
		}
	}
}

#[cfg(any(
	target_os = "macos",
	target_os = "windows",
	all(target_os = "linux", not(target_env = "ohos"))
))]
pub use desktop::SecureStore;

/// Browser environments have no supported OS credential-store backend.
#[cfg(all(target_arch = "wasm32", target_os = "unknown", not(feature = "guest")))]
#[expect(
	clippy::unused_async,
	reason = "the stubs stay async so browser and native callers share one API"
)]
pub mod web {
	use super::*;
	/// Always unavailable; localStorage/IndexedDB are not used for secrets.
	pub fn available() -> bool {
		false
	}
	/// Returns unavailable without persisting anything.
	pub async fn set(_key: Key<'_>, _secret: &[u8]) -> Result<(), Error> {
		Err(Error::Unavailable)
	}
	/// Returns unavailable, not a fabricated missing entry.
	pub async fn get(_key: Key<'_>) -> Result<Option<Bytes>, Error> {
		Err(Error::Unavailable)
	}
	/// Returns unavailable without changing origin storage.
	pub async fn delete(_key: Key<'_>) -> Result<(), Error> {
		Err(Error::Unavailable)
	}
}

/// Typed native guest client; resolves its numeric plugin ID once.
#[cfg(all(target_arch = "wasm32", feature = "guest"))]
pub mod guest {
	use leptoskit_bridge::guest::Client;

	use super::*;

	/// Cached native secure-store connection.
	#[derive(Debug, Clone, Copy)]
	pub struct SecureStore(Client);
	impl SecureStore {
		/// Connects to the installed, granted native plugin.
		pub fn new() -> Result<Self, Error> {
			Client::new(NAME).map(Self)
		}

		/// Reads a binary secret, distinguishing absence from an empty secret.
		pub async fn get(self, key: Key<'_>) -> Result<Option<Bytes>, Error> {
			let response = self.0.call(GET, &key.encode(None)?)?.await?;
			Ok(decode_secret(&response)?.map(|_| response.slice(4..)))
		}

		/// Stores a bounded binary secret.
		pub async fn set(self, key: Key<'_>, secret: &[u8]) -> Result<(), Error> {
			let response = self.0.call(SET, &key.encode(Some(secret))?)?.await?;

			if !response.is_empty() {
				return Err(Error::Native);
			}

			Ok(())
		}

		/// Deletes a secret. Deleting an absent key succeeds.
		pub async fn delete(self, key: Key<'_>) -> Result<(), Error> {
			let response = self.0.call(DELETE, &key.encode(None)?)?.await?;

			if !response.is_empty() {
				return Err(Error::Native);
			}

			Ok(())
		}
	}
}

/// `leptoskit_use::AsyncStorage` adapter over the credential stores.
#[cfg(all(
	feature = "async-storage",
	any(
		any(
			target_os = "macos",
			target_os = "windows",
			all(target_os = "linux", not(target_env = "ohos"))
		),
		all(target_arch = "wasm32", feature = "guest")
	)
))]
mod async_storage;
#[cfg(all(
	feature = "async-storage",
	any(
		any(
			target_os = "macos",
			target_os = "windows",
			all(target_os = "linux", not(target_env = "ohos"))
		),
		all(target_arch = "wasm32", feature = "guest")
	)
))]
pub use async_storage::Backend;
#[cfg(all(
	feature = "async-storage",
	any(
		any(
			target_os = "macos",
			target_os = "windows",
			all(target_os = "linux", not(target_env = "ohos"))
		),
		all(target_arch = "wasm32", feature = "guest")
	)
))]
pub use async_storage::SecureStorage;
#[cfg(all(
	feature = "async-storage",
	any(
		any(
			target_os = "macos",
			target_os = "windows",
			all(target_os = "linux", not(target_env = "ohos"))
		),
		all(target_arch = "wasm32", feature = "guest")
	)
))]
pub use async_storage::SecureStorageError;

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn validates_names_and_value_limits() {
		for name in ["", "a/b", "a\0b", "é", "a b"] {
			assert!(Key::new("test", name).is_err());
		}

		assert!(Key::new(&"a".repeat(MAX_NAME_BYTES + 1), "key").is_err());
		let key = Key::new("login", "token").unwrap();
		assert_eq!(
			key.encode(Some(&vec![0; MAX_SECRET_BYTES + 1])),
			Err(Error::TooLarge)
		);
		let payload = key.encode(Some(&[0, 255, 128])).unwrap();

		let request = Request::decode(SET, &payload).unwrap();
		assert_eq!(request.key, key);
		assert_eq!(request.secret, Some([0, 255, 128].as_slice()));
	}
	#[test]
	fn framing_rejects_truncation_trailers_and_wrong_method() {
		let payload = Key::new("service", "key")
			.unwrap()
			.encode(Some(b"secret"))
			.unwrap();

		for length in 0..payload.len() {
			assert!(Request::decode(SET, &payload[..length]).is_err());
		}

		let mut trailing = payload.to_vec();
		trailing.push(0);
		assert!(Request::decode(SET, &trailing).is_err());
		assert!(Request::decode(GET, &payload).is_err());
		assert!(matches!(
			Request::decode(MethodId(9), &[]),
			Err(Error::Unavailable)
		));
	}
	#[test]
	fn absent_and_empty_are_distinct_and_malformed_replies_fail() {
		assert_eq!(decode_secret(&[0, 0, 0, 0]), Ok(None));
		assert_eq!(decode_secret(&[1, 0, 0, 0]), Ok(Some([].as_slice())));
		assert_eq!(
			decode_secret(&[1, 0, 0, 0, 255]),
			Ok(Some([255].as_slice()))
		);

		for input in [&[0][..], &[2, 0, 0, 0], &[0, 0, 0, 0, 1]] {
			assert_eq!(decode_secret(input), Err(Error::Native));
		}
	}

	#[test]
	fn oversized_wire_values_fail_before_storage() {
		let mut payload = Encoder::with_capacity(MAX_SECRET_BYTES + 32);
		payload.bytes(b"login").unwrap();
		payload.bytes(b"token").unwrap();
		payload.bytes(&vec![0; MAX_SECRET_BYTES + 1]).unwrap();
		assert!(matches!(
			Request::decode(SET, &payload.finish()),
			Err(Error::TooLarge)
		));
		let mut response = vec![0; MAX_SECRET_BYTES + 5];
		response[0] = 1;
		assert_eq!(decode_secret(&response), Err(Error::Native));
	}
}
