use std::path::Path;
use std::path::PathBuf;

use cap_std::fs::Dir;
use leptoskit_bridge::Plugin;
use leptoskit_plugin_filesystem::Filesystem;

use super::*;

const FILE: &str = "settings.json";

struct Storage {
	root: Dir,
	files: Filesystem,
}

/// Application settings. All methods may perform disk I/O and belong on a
/// worker. Construction only captures the trusted host path; first use opens
/// a directory capability and loads the document.
pub struct Store {
	data_dir: PathBuf,
	storage: Option<Storage>,
	entries: Entries,
	loaded: bool,
}

/// Factory used by generated hosts. No filesystem access occurs until the
/// registered Rust worker invokes the first operation.
pub fn from_app_dirs(data_dir: &Path, _cache_dir: &Path) -> Result<Store, Error> {
	Ok(Store::new(data_dir))
}

fn failure(error: &io::Error) -> Error {
	match error.kind() {
		io::ErrorKind::PermissionDenied => Error::PermissionDenied,
		io::ErrorKind::InvalidInput | io::ErrorKind::InvalidData => Error::InvalidArgument,
		_ => Error::Native,
	}
}

impl Store {
	/// Captures a trusted application data root. Guests cannot supply or change
	/// it.
	pub fn new(data_dir: impl Into<PathBuf>) -> Self {
		Self {
			data_dir: data_dir.into(),
			storage: None,
			entries: Entries::new(),
			loaded: false,
		}
	}

	fn storage(&mut self) -> Result<&Storage, Error> {
		if self.storage.is_none() {
			let parent = Dir::open_ambient_dir(&self.data_dir, cap_std::ambient_authority())
				.map_err(|error| failure(&error))?;
			let mut builder = cap_std::fs::DirBuilder::new();
			builder.recursive(true);
			#[cfg(unix)]
			{
				use cap_std::fs::DirBuilderExt as _;
				builder.mode(0o700);
			}
			parent
				.create_dir_with("store", &builder)
				.map_err(|error| failure(&error))?;
			let root = parent.open_dir("store").map_err(|error| failure(&error))?;
			#[cfg(unix)]
			{
				use cap_std::fs::PermissionsExt as _;
				root.set_permissions(".", cap_std::fs::Permissions::from_mode(0o700))
					.map_err(|error| failure(&error))?;
			}
			let files = Filesystem::from_dir(root.try_clone().map_err(|error| failure(&error))?);
			self.storage = Some(Storage { root, files });
		}
		self.storage.as_ref().ok_or(Error::Native)
	}

	fn ensure_loaded(&mut self) -> Result<(), Error> {
		if !self.loaded {
			self.load()?;
		}
		Ok(())
	}

	/// Reloads the saved document. A missing file means an empty store.
	/// Malformed data fails without changing memory; successful reload
	/// discards staged edits.
	pub fn load(&mut self) -> Result<(), Error> {
		let storage = self.storage()?;
		let next = match storage.root.symlink_metadata(FILE) {
			Ok(_) => decode(&storage.files.read(FILE)?)?,
			Err(error) if error.kind() == io::ErrorKind::NotFound => Entries::new(),
			Err(error) => return Err(failure(&error)),
		};
		self.entries = next;
		self.loaded = true;
		Ok(())
	}

	/// Returns a value, distinguishing missing keys from JSON `null`.
	pub fn get(&mut self, name: &str) -> Result<Option<&Value>, Error> {
		key(name)?;
		self.ensure_loaded()?;
		Ok(self.entries.get(name))
	}

	/// Returns the current staged entries in key order.
	pub fn entries(&mut self) -> Result<&Entries, Error> {
		self.ensure_loaded()?;
		Ok(&self.entries)
	}

	/// Stages one value. Call [`Self::save`] explicitly to persist it.
	pub fn set(&mut self, name: &str, value: Value) -> Result<(), Error> {
		self.apply(&[Change::Set(name.into(), value)], Save::Deferred)
	}

	/// Stages removal of one key; missing keys are harmless.
	pub fn remove(&mut self, name: &str) -> Result<(), Error> {
		self.apply(&[Change::Remove(name.into())], Save::Deferred)
	}

	/// Stages removal of every key.
	pub fn clear(&mut self) -> Result<(), Error> {
		self.apply(&[Change::Clear], Save::Deferred)
	}

	/// Validates the entire candidate before publishing it. Committed batches
	/// use one sync and atomic replacement, then update memory. A failed commit
	/// preserves the previous staged state and saved document.
	pub fn apply(&mut self, changes: &[Change], save: Save) -> Result<(), Error> {
		self.ensure_loaded()?;
		let (next, bytes) = candidate(&self.entries, changes)?;
		if save == Save::Commit {
			self.storage()?.files.write(FILE, &bytes)?;
		}
		self.entries = next;
		Ok(())
	}

	/// Saves staged changes once. A failed save retains the staged changes for
	/// retry. File data is synced; directory-entry durability on power loss is
	/// not promised. No save occurs implicitly on drop or process shutdown.
	pub fn save(&mut self) -> Result<(), Error> {
		self.ensure_loaded()?;
		let bytes = encode(&self.entries)?;
		self.storage()?.files.write(FILE, &bytes)
	}
}

impl Plugin for Store {
	fn name(&self) -> &'static str {
		NAME
	}

	fn methods(&self) -> &'static [MethodId] {
		METHODS
	}

	fn invoke(&mut self, method: MethodId, payload: Bytes) -> Result<Bytes, Error> {
		if payload.len() > MAX_STORE_BYTES {
			return Err(Error::TooLarge);
		}
		match method {
			GET => {
				let mut input = Decoder::new(&payload);
				let name = input.string()?;
				input.finish()?;
				let mut output = Encoder::with_capacity(4);
				if let Some(value) = self.get(name)? {
					output.u32(1);
					output.bytes(&value_json(value)?)?;
				} else {
					output.u32(0);
				}
				Ok(output.finish())
			}
			ENTRIES | SAVE | LOAD => {
				if !payload.is_empty() {
					return Err(Error::InvalidArgument);
				}
				match method {
					ENTRIES => Ok(encode(self.entries()?)?.into()),
					SAVE => {
						self.save()?;
						Ok(Bytes::new())
					}
					_ => {
						self.load()?;
						Ok(Bytes::new())
					}
				}
			}
			APPLY => {
				let (changes, save) = decode_changes(&payload)?;
				self.apply(&changes, save)?;
				Ok(Bytes::new())
			}
			_ => Err(Error::Unavailable),
		}
	}
}

#[cfg(test)]
mod tests {
	use serde_json::json;

	use super::*;

	#[test]
	fn stages_batches_saves_reloads_and_clears() {
		let root = tempfile::tempdir().unwrap();
		let mut store = Store::new(root.path());
		assert!(
			!root.path().join("store").exists(),
			"construction performs no I/O"
		);
		store
			.set("theme", json!({"name":"dark","contrast":true}))
			.unwrap();
		assert!(!root.path().join("store/settings.json").exists());
		store.save().unwrap();
		let mut restored = Store::new(root.path());
		assert_eq!(
			restored.get("theme").unwrap(),
			Some(&json!({"name":"dark","contrast":true}))
		);
		store
			.apply(
				&[
					Change::Set("launches".into(), json!(7)),
					Change::Remove("theme".into()),
					Change::Set("nullable".into(), Value::Null),
				],
				Save::Commit,
			)
			.unwrap();
		restored.load().unwrap();
		assert_eq!(restored.get("theme").unwrap(), None);
		assert_eq!(restored.get("launches").unwrap(), Some(&json!(7)));
		assert_eq!(restored.get("nullable").unwrap(), Some(&Value::Null));
		restored.clear().unwrap();
		restored.load().unwrap();
		assert_eq!(
			restored.entries().unwrap().len(),
			2,
			"load discards staged clear"
		);
		restored.clear().unwrap();
		restored.save().unwrap();
		assert!(Store::new(root.path()).entries().unwrap().is_empty());
	}

	#[test]
	fn malformed_reload_and_rejected_transaction_preserve_staged_state() {
		let root = tempfile::tempdir().unwrap();
		let mut store = Store::new(root.path());
		store
			.apply(&[Change::Set("keep".into(), json!(1))], Save::Commit)
			.unwrap();
		let saved = std::fs::read(root.path().join("store/settings.json")).unwrap();
		assert_eq!(
			store.apply(
				&[Change::Clear, Change::Set(String::new(), Value::Null)],
				Save::Commit
			),
			Err(Error::InvalidArgument)
		);
		assert_eq!(
			std::fs::read(root.path().join("store/settings.json")).unwrap(),
			saved
		);
		std::fs::write(root.path().join("store/settings.json"), b"{bad json").unwrap();
		assert_eq!(store.load(), Err(Error::InvalidArgument));
		assert_eq!(store.get("keep").unwrap(), Some(&json!(1)));
	}

	#[cfg(unix)]
	#[test]
	fn failed_atomic_write_keeps_memory_and_saved_file() {
		use std::os::unix::fs::PermissionsExt as _;
		let root = tempfile::tempdir().unwrap();
		let mut store = Store::new(root.path());
		store
			.apply(&[Change::Set("keep".into(), json!(1))], Save::Commit)
			.unwrap();
		let saved = std::fs::read(root.path().join("store/settings.json")).unwrap();
		let directory = root.path().join("store");
		std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o500)).unwrap();
		let result = store.apply(&[Change::Set("keep".into(), json!(2))], Save::Commit);
		std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700)).unwrap();
		assert_eq!(result, Err(Error::PermissionDenied));
		assert_eq!(store.get("keep").unwrap(), Some(&json!(1)));
		assert_eq!(
			std::fs::read(directory.join("settings.json")).unwrap(),
			saved
		);
	}

	#[cfg(unix)]
	#[test]
	fn store_rejects_symlink_escape() {
		let root = tempfile::tempdir().unwrap();
		let outside = tempfile::tempdir().unwrap();
		std::fs::create_dir(root.path().join("store")).unwrap();
		let secret = outside.path().join("settings.json");
		std::fs::write(&secret, br#"{"version":1,"entries":{"private":true}}"#).unwrap();
		std::os::unix::fs::symlink(&secret, root.path().join("store/settings.json")).unwrap();
		assert_eq!(Store::new(root.path()).load(), Err(Error::PermissionDenied));
	}
}
