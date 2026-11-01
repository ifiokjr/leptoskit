//! Structured logging without a global logger or remote backend dependency.
//! The binary guest protocol is bounded independently from JSON-lines output.
use std::collections::BTreeMap;

use leptoskit_bridge::Bytes;
use leptoskit_bridge::Error;
use leptoskit_bridge::MethodId;
use leptoskit_bridge::codec::Decoder;
use leptoskit_bridge::codec::Encoder;
#[cfg(any(not(target_arch = "wasm32"), not(feature = "guest")))]
use serde::Serialize;

/// Stable plugin name.
pub const NAME: &str = "logging";
/// Binary record -> accepted u32 (0 filtered, 1 written).
pub const WRITE: MethodId = MethodId(0);
/// Empty -> flush native file data to storage.
pub const FLUSH: MethodId = MethodId(1);
/// Empty -> flush and release resources; terminal and idempotent.
pub const CLOSE: MethodId = MethodId(2);
/// Supported methods.
pub const METHODS: &[MethodId] = &[WRITE, FLUSH, CLOSE];
/// Maximum message UTF-8 bytes.
pub const MAX_MESSAGE_BYTES: usize = 16 * 1024;
/// Maximum target UTF-8 bytes.
pub const MAX_TARGET_BYTES: usize = 128;
/// Maximum structured string fields.
pub const MAX_FIELDS: usize = 32;
/// Maximum field key bytes.
pub const MAX_FIELD_KEY_BYTES: usize = 64;
/// Maximum field value bytes.
pub const MAX_FIELD_VALUE_BYTES: usize = 1024;
/// Maximum complete binary input.
pub const MAX_RECORD_BYTES: usize = 64 * 1024;

/// Ascending severity, shared with the binary protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Level {
	/// Fine-grained diagnostic information.
	Trace = 0,
	/// Developer diagnostic information.
	Debug = 1,
	/// Normal application events.
	Info = 2,
	/// An unusual but recoverable condition.
	Warn = 3,
	/// An operation failed.
	Error = 4,
}

impl Level {
	fn decode(value: u32) -> Result<Self, Error> {
		match value {
			0 => Ok(Self::Trace),
			1 => Ok(Self::Debug),
			2 => Ok(Self::Info),
			3 => Ok(Self::Warn),
			4 => Ok(Self::Error),
			_ => Err(Error::InvalidArgument),
		}
	}

	/// Stable lowercase severity label.
	pub fn as_str(self) -> &'static str {
		match self {
			Self::Trace => "trace",
			Self::Debug => "debug",
			Self::Info => "info",
			Self::Warn => "warn",
			Self::Error => "error",
		}
	}
}

/// One event. Targets identify components and are never filesystem paths.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
	/// Event severity.
	pub level: Level,
	/// Nonempty component label.
	pub target: String,
	/// Human-readable message; newlines are safely JSON-escaped in files.
	pub message: String,
	/// Ordered structured string metadata. Never log credentials here.
	pub fields: BTreeMap<String, String>,
}

fn label(value: &str, maximum: usize) -> Result<(), Error> {
	if value.len() > maximum {
		return Err(Error::TooLarge);
	}

	if value.is_empty() || value.chars().any(char::is_control) {
		return Err(Error::InvalidArgument);
	}

	Ok(())
}

impl Record {
	/// Validates an event without allocating an encoded copy.
	pub fn validate(&self) -> Result<(), Error> {
		label(&self.target, MAX_TARGET_BYTES)?;

		if self.message.len() > MAX_MESSAGE_BYTES || self.fields.len() > MAX_FIELDS {
			return Err(Error::TooLarge);
		}

		let mut bytes = 16 + self.target.len() + self.message.len();

		for (key, value) in &self.fields {
			label(key, MAX_FIELD_KEY_BYTES)?;

			if value.len() > MAX_FIELD_VALUE_BYTES {
				return Err(Error::TooLarge);
			}

			bytes += 8 + key.len() + value.len();
		}

		if bytes > MAX_RECORD_BYTES {
			return Err(Error::TooLarge);
		}

		Ok(())
	}

	/// Encodes a typed record into the bounded native protocol.
	pub fn encode(&self) -> Result<Bytes, Error> {
		self.validate()?;
		let mut output = Encoder::with_capacity(16 + self.target.len() + self.message.len());
		output.u32(self.level as u32);
		output.bytes(self.target.as_bytes())?;
		output.bytes(self.message.as_bytes())?;
		output.u32(u32::try_from(self.fields.len()).map_err(|_| Error::TooLarge)?);

		for (key, value) in &self.fields {
			output.bytes(key.as_bytes())?;

			output.bytes(value.as_bytes())?;
		}

		Ok(output.finish())
	}

	/// Decodes every field, rejecting duplicate keys, oversized or trailing
	/// data.
	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		if bytes.len() > MAX_RECORD_BYTES {
			return Err(Error::TooLarge);
		}

		let mut input = Decoder::new(bytes);
		let level = Level::decode(input.u32()?)?;
		let target = input.string()?;
		label(target, MAX_TARGET_BYTES)?;
		let message = input.string()?;

		if message.len() > MAX_MESSAGE_BYTES {
			return Err(Error::TooLarge);
		}

		let count = input.u32()? as usize;

		if count > MAX_FIELDS {
			return Err(Error::TooLarge);
		}

		let mut fields = BTreeMap::new();

		for _ in 0..count {
			let key = input.string()?;
			label(key, MAX_FIELD_KEY_BYTES)?;
			let value = input.string()?;

			if value.len() > MAX_FIELD_VALUE_BYTES {
				return Err(Error::TooLarge);
			}

			if fields.insert(key.into(), value.into()).is_some() {
				return Err(Error::InvalidArgument);
			}
		}

		input.finish()?;
		let record = Self {
			level,
			target: target.into(),
			message: message.into(),
			fields,
		};
		record.validate()?;
		Ok(record)
	}
}

#[cfg(any(not(target_arch = "wasm32"), not(feature = "guest")))]
fn json(record: &Record, timestamp_ms: u64) -> Result<Vec<u8>, Error> {
	record.validate()?;
	#[derive(Serialize)]
	struct Entry<'a> {
		timestamp_ms: u64,
		level: &'static str,
		target: &'a str,
		message: &'a str,
		fields: &'a BTreeMap<String, String>,
	}
	let mut output = serde_json::to_vec(&Entry {
		timestamp_ms,
		level: record.level.as_str(),
		target: &record.target,
		message: &record.message,
		fields: &record.fields,
	})
	.map_err(|_| Error::InvalidArgument)?;
	output.push(b'\n');
	Ok(output)
}

#[cfg(not(target_arch = "wasm32"))]
mod native;
#[cfg(not(target_arch = "wasm32"))]
pub use native::Config;
#[cfg(not(target_arch = "wasm32"))]
pub use native::Logger;
#[cfg(not(target_arch = "wasm32"))]
pub use native::from_app_dirs;

/// Browser console adapter.
#[cfg(all(target_arch = "wasm32", target_os = "unknown", not(feature = "guest")))]
pub mod web;

/// Cached guest client for native logging.
#[cfg(all(target_arch = "wasm32", feature = "guest"))]
pub mod guest {
	use super::*;
	/// One host logging plugin. Closing affects all copies of this client.
	#[derive(Debug, Clone, Copy)]
	pub struct Logger(leptoskit_bridge::guest::Client);
	impl Logger {
		/// Resolves the logging plugin once.
		pub fn new() -> Result<Self, Error> {
			leptoskit_bridge::guest::Client::new(NAME).map(Self)
		}

		/// Returns false when the host severity threshold filters the record.
		pub async fn write(self, record: &Record) -> Result<bool, Error> {
			let result = self.0.call(WRITE, &record.encode()?)?.await?;
			let mut input = Decoder::new(&result);

			let accepted = match input.u32()? {
				0 => false,
				1 => true,
				_ => return Err(Error::InvalidArgument),
			};

			input.finish()?;
			Ok(accepted)
		}

		/// Requests an explicit durable file flush.
		pub async fn flush(self) -> Result<(), Error> {
			self.empty(FLUSH).await
		}

		/// Flushes and closes the host logger permanently.
		pub async fn close(self) -> Result<(), Error> {
			self.empty(CLOSE).await
		}

		async fn empty(self, method: MethodId) -> Result<(), Error> {
			if self.0.call(method, &[])?.await?.is_empty() {
				Ok(())
			} else {
				Err(Error::InvalidArgument)
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	fn record() -> Record {
		Record {
			level: Level::Warn,
			target: "tests".into(),
			message: "synthetic\nmessage".into(),
			fields: BTreeMap::from([("case".into(), "codec".into())]),
		}
	}
	#[test]
	fn record_roundtrip_and_malformed_wire_validation() {
		let record = record();
		let bytes = record.encode().unwrap();
		assert_eq!(Record::decode(&bytes), Ok(record));

		for length in 0..bytes.len() {
			assert!(Record::decode(&bytes[..length]).is_err());
		}

		let mut bytes = bytes.to_vec();
		bytes.push(0);
		assert_eq!(Record::decode(&bytes), Err(Error::InvalidArgument));

		bytes[0] = 5;
		assert_eq!(Record::decode(&bytes), Err(Error::InvalidArgument));
		let mut duplicate = Encoder::with_capacity(40);
		duplicate.u32(2);
		duplicate.bytes(b"tests").unwrap();
		duplicate.bytes(b"message").unwrap();
		duplicate.u32(2);

		for _ in 0..2 {
			duplicate.bytes(b"same").unwrap();
			duplicate.bytes(b"value").unwrap();
		}

		assert_eq!(
			Record::decode(&duplicate.finish()),
			Err(Error::InvalidArgument)
		);
	}
	#[test]
	fn records_reject_oversized_and_ambiguous_labels() {
		let mut record = record();
		record.target = "\nspoof".into();
		assert_eq!(record.validate(), Err(Error::InvalidArgument));
		record.target = "x".repeat(MAX_TARGET_BYTES + 1);
		assert_eq!(record.validate(), Err(Error::TooLarge));
		record.target = "tests".into();
		record.message = "x".repeat(MAX_MESSAGE_BYTES + 1);
		assert_eq!(record.validate(), Err(Error::TooLarge));
		record.message.clear();
		record.fields = (0..=MAX_FIELDS)
			.map(|index| (index.to_string(), "value".into()))
			.collect();
		assert_eq!(record.validate(), Err(Error::TooLarge));
	}
}
