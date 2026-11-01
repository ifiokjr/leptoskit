//! Native-hosted guest client. Clone the cached identifier cheaply.
use leptoskit_bridge::guest::Client;

use super::*;

/// The host's single application settings store.
#[derive(Debug, Clone, Copy)]
pub struct Store(Client);

impl Store {
	/// Resolves the registered store once.
	pub fn new() -> Result<Self, Error> {
		Client::new(NAME).map(Self)
	}

	/// Returns JSON `null` as `Some(Value::Null)`; absence is `None`.
	pub async fn get(self, name: &str) -> Result<Option<Value>, Error> {
		key(name)?;
		let mut input = Encoder::with_capacity(name.len() + 4);
		input.bytes(name.as_bytes())?;
		let bytes = self.0.call(GET, &input.finish())?.await?;

		if bytes.len() > MAX_VALUE_BYTES + 8 {
			return Err(Error::TooLarge);
		}

		let mut output = Decoder::new(&bytes);

		let value = match output.u32()? {
			0 => None,
			1 => Some(read_value(output.bytes()?)?),
			_ => return Err(Error::InvalidArgument),
		};

		output.finish()?;
		Ok(value)
	}

	/// Returns every staged entry in key order.
	pub async fn entries(self) -> Result<Entries, Error> {
		decode(&self.0.call(ENTRIES, &[])?.await?)
	}

	/// Stages a value; persistence is explicit.
	pub async fn set(self, name: &str, value: Value) -> Result<(), Error> {
		self.apply(&[Change::Set(name.into(), value)], Save::Deferred)
			.await
	}

	/// Stages removal of one key.
	pub async fn remove(self, name: &str) -> Result<(), Error> {
		self.apply(&[Change::Remove(name.into())], Save::Deferred)
			.await
	}

	/// Stages removal of every key.
	pub async fn clear(self) -> Result<(), Error> {
		self.apply(&[Change::Clear], Save::Deferred).await
	}

	/// Applies an ordered batch atomically; `Commit` saves once before
	/// publishing.
	pub async fn apply(self, changes: &[Change], save: Save) -> Result<(), Error> {
		let reply = self.0.call(APPLY, &encode_changes(changes, save)?)?.await?;

		if reply.is_empty() {
			Ok(())
		} else {
			Err(Error::InvalidArgument)
		}
	}

	/// Persists all staged changes once.
	pub async fn save(self) -> Result<(), Error> {
		let reply = self.0.call(SAVE, &[])?.await?;

		if reply.is_empty() {
			Ok(())
		} else {
			Err(Error::InvalidArgument)
		}
	}

	/// Reloads saved data and discards staged changes on successful validation.
	pub async fn load(self) -> Result<(), Error> {
		let reply = self.0.call(LOAD, &[])?.await?;

		if reply.is_empty() {
			Ok(())
		} else {
			Err(Error::InvalidArgument)
		}
	}
}
