//! Synchronous localStorage adapter. The entire bounded document uses one
//! atomic setItem. Browser quota and persistence policy still apply.
use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;

use super::*;

/// One settings namespace per application origin.
pub const STORAGE_KEY: &str = "leptoskit:store:v1";

fn failure(error: &JsValue) -> Error {
	match error
		.dyn_ref::<web_sys::DomException>()
		.map(web_sys::DomException::name)
		.as_deref()
	{
		Some("SecurityError" | "NotAllowedError") => Error::PermissionDenied,
		Some("QuotaExceededError") => Error::TooLarge,
		Some("NotSupportedError") => Error::Unavailable,
		_ => Error::Native,
	}
}

/// One in-memory view of origin-local settings. Independent instances/tabs
/// use last successful save wins; reload explicitly to observe external edits.
pub struct Store {
	storage: web_sys::Storage,
	entries: Entries,
}

impl Store {
	/// Opens localStorage and validates the saved document.
	pub fn open() -> Result<Self, Error> {
		let storage = web_sys::window()
			.ok_or(Error::Unavailable)?
			.local_storage()
			.map_err(|error| failure(&error))?
			.ok_or(Error::Unavailable)?;
		let mut store = Self {
			storage,
			entries: Entries::new(),
		};
		store.load()?;
		Ok(store)
	}

	/// Returns JSON null distinctly from a missing key.
	pub fn get(&self, name: &str) -> Result<Option<&Value>, Error> {
		key(name)?;
		Ok(self.entries.get(name))
	}

	/// Current staged entries in key order.
	pub fn entries(&self) -> &Entries {
		&self.entries
	}

	/// Stages one value; call [`Self::save`] explicitly.
	pub fn set(&mut self, name: &str, value: Value) -> Result<(), Error> {
		self.apply(&[Change::Set(name.into(), value)], Save::Deferred)
	}

	/// Stages deletion of one key.
	pub fn remove(&mut self, name: &str) -> Result<(), Error> {
		self.apply(&[Change::Remove(name.into())], Save::Deferred)
	}

	/// Stages deletion of every key.
	pub fn clear(&mut self) -> Result<(), Error> {
		self.apply(&[Change::Clear], Save::Deferred)
	}

	/// Applies the full candidate or nothing. A quota/storage error during
	/// commit leaves both memory and the prior localStorage document
	/// unchanged.
	pub fn apply(&mut self, changes: &[Change], save: Save) -> Result<(), Error> {
		let (next, bytes) = candidate(&self.entries, changes)?;
		if save == Save::Commit {
			self.write(&bytes)?;
		}
		self.entries = next;
		Ok(())
	}

	fn write(&self, bytes: &[u8]) -> Result<(), Error> {
		let value = std::str::from_utf8(bytes).map_err(|_| Error::InvalidArgument)?;
		self.storage
			.set_item(STORAGE_KEY, value)
			.map_err(|error| failure(&error))
	}

	/// Performs one synchronous localStorage write. No implicit save occurs on
	/// drop.
	pub fn save(&self) -> Result<(), Error> {
		self.write(&encode(&self.entries)?)
	}

	/// Reloads storage, discarding staged changes only on successful
	/// validation.
	pub fn load(&mut self) -> Result<(), Error> {
		let bytes = self
			.storage
			.get_item(STORAGE_KEY)
			.map_err(|error| failure(&error))?;
		let next = bytes.map_or_else(|| Ok(Entries::new()), |value| decode(value.as_bytes()))?;
		self.entries = next;
		Ok(())
	}
}
