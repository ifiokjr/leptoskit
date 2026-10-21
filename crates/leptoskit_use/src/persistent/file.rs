use std::fs;
use std::io::Write;
use std::marker::PhantomData;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;

use serde::Serialize;
use serde::de::DeserializeOwned;
use sha2::Digest;
use sha2::Sha256;
use tempfile::NamedTempFile;

use crate::AsyncStorage;
use crate::Storage;
use crate::StorageError;
use crate::StorageFuture;

/// Persistent JSON storage in a host-selected native application directory.
///
/// Keys are hashed into filenames, so empty, long, Unicode, and path-like keys
/// remain inside the directory. Writes replace a key atomically. Separate
/// instances reopen the same data. Async methods perform the complete
/// filesystem operation on a blocking worker, without requiring Tokio or
/// another runtime. The host must select an application-private directory and
/// retain it across launches. On mobile, supply the platform's application data
/// directory.
#[derive(Debug, Clone)]
pub struct FileStorage<T> {
	directory: Arc<Path>,
	value: PhantomData<fn() -> T>,
}

impl<T> FileStorage<T> {
	/// Open or create a directory for persistent values.
	pub fn new(directory: impl AsRef<Path>) -> Result<Self, StorageError> {
		fs::create_dir_all(directory.as_ref())?;

		Ok(Self {
			directory: directory.as_ref().into(),
			value: PhantomData,
		})
	}

	fn path(&self, key: &str) -> PathBuf {
		self.directory
			.join(format!("{:x}.json", Sha256::digest(key.as_bytes())))
	}
}

impl<T> Storage<T> for FileStorage<T>
where
	T: Serialize + DeserializeOwned + Send + Sync + 'static,
{
	type Error = StorageError;

	fn get(&self, key: &str) -> Result<Option<T>, StorageError> {
		match fs::read(self.path(key)) {
			Ok(bytes) => Ok(Some(serde_json::from_slice(&bytes)?)),
			Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
			Err(error) => Err(error.into()),
		}
	}

	fn set(&self, key: &str, value: &T) -> Result<(), StorageError> {
		let bytes = serde_json::to_vec(value)?;
		let mut file = NamedTempFile::new_in(&self.directory)?;
		file.write_all(&bytes)?;
		file.as_file().sync_all()?;
		file.persist(self.path(key))
			.map_err(|error| StorageError::Io(error.error))?;
		Ok(())
	}

	fn remove(&self, key: &str) -> Result<(), StorageError> {
		match fs::remove_file(self.path(key)) {
			Ok(()) => Ok(()),
			Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
			Err(error) => Err(error.into()),
		}
	}
}

impl<T> AsyncStorage<T> for FileStorage<T>
where
	T: Clone + Serialize + DeserializeOwned + Send + Sync + 'static,
{
	type Error = StorageError;

	fn get_async(&self, key: String) -> StorageFuture<'_, Result<Option<T>, StorageError>> {
		let storage = self.clone();
		Box::pin(blocking::unblock(move || storage.get(&key)))
	}

	fn set_async(&self, key: String, value: T) -> StorageFuture<'_, Result<(), StorageError>> {
		let storage = self.clone();
		Box::pin(blocking::unblock(move || storage.set(&key, &value)))
	}

	fn remove_async(&self, key: String) -> StorageFuture<'_, Result<(), StorageError>> {
		let storage = self.clone();
		Box::pin(blocking::unblock(move || storage.remove(&key)))
	}
}
