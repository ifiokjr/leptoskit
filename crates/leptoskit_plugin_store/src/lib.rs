//! Bounded JSON settings with explicit persistence and atomic batches.
//!
//! Values are JSON; the RPC envelope is binary. Native storage is confined to
//! one app-owned file. Generated hosts run all operations on a Rust worker.

use std::collections::BTreeMap;
use std::io::Write;
use std::io::{self};

use leptoskit_bridge::Bytes;
use leptoskit_bridge::Error;
use leptoskit_bridge::MethodId;
use leptoskit_bridge::codec::Decoder;
use leptoskit_bridge::codec::Encoder;
use serde::Deserialize;
use serde::Serialize;
pub use serde_json::Value;

/// Runtime plugin name.
pub const NAME: &str = "store";
/// Length-prefixed key -> presence u32 and, when present, length-prefixed JSON.
pub const GET: MethodId = MethodId(0);
/// Empty -> persisted document containing all current entries.
pub const ENTRIES: MethodId = MethodId(1);
/// Save mode, change count, then binary tagged changes -> empty.
pub const APPLY: MethodId = MethodId(2);
/// Empty -> persist staged entries with one atomic replacement.
pub const SAVE: MethodId = MethodId(3);
/// Empty -> reload, discarding staged changes only on successful validation.
pub const LOAD: MethodId = MethodId(4);
/// Supported methods.
pub const METHODS: &[MethodId] = &[GET, ENTRIES, APPLY, SAVE, LOAD];
/// Maximum UTF-8 bytes in one nonempty key.
pub const MAX_KEY_BYTES: usize = 256;
/// Maximum serialized JSON bytes in one value.
pub const MAX_VALUE_BYTES: usize = 64 * 1024;
/// Maximum serialized document and RPC input.
pub const MAX_STORE_BYTES: usize = 512 * 1024;
/// Maximum retained keys.
pub const MAX_ENTRIES: usize = 1024;
/// Maximum mutations in one batch.
pub const MAX_CHANGES: usize = 256;
/// Maximum nesting depth in a value.
pub const MAX_DEPTH: usize = 64;

/// Sorted application settings.
pub type Entries = BTreeMap<String, Value>;

/// Ordered mutations. A batch either applies completely or leaves state intact.
#[derive(Debug, Clone, PartialEq)]
pub enum Change {
	/// Insert or replace a key.
	Set(String, Value),
	/// Delete a key; a missing key is harmless.
	Remove(String),
	/// Remove all keys.
	Clear,
}

/// Explicit persistence policy; no hidden autosave timer runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Save {
	/// Stage in memory until [`SAVE`] or a committed batch.
	Deferred,
	/// Persist the entire candidate state once before publishing it in memory.
	Commit,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
	version: u32,
	#[serde(deserialize_with = "unique_entries")]
	entries: Entries,
}

fn unique_entries<'de, D: serde::Deserializer<'de>>(input: D) -> Result<Entries, D::Error> {
	struct Visitor;
	impl<'de> serde::de::Visitor<'de> for Visitor {
		type Value = Entries;

		fn expecting(&self, output: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
			output.write_str("an object with unique setting keys")
		}

		fn visit_map<M: serde::de::MapAccess<'de>>(self, mut map: M) -> Result<Entries, M::Error> {
			let mut entries = Entries::new();
			while let Some((key, value)) = map.next_entry::<String, Value>()? {
				if entries.insert(key, value).is_some() {
					return Err(serde::de::Error::custom("duplicate setting key"));
				}
			}
			Ok(entries)
		}
	}
	input.deserialize_map(Visitor)
}

pub(crate) fn key(key: &str) -> Result<(), Error> {
	if key.len() > MAX_KEY_BYTES {
		return Err(Error::TooLarge);
	}
	if key.is_empty() || key.chars().any(char::is_control) {
		return Err(Error::InvalidArgument);
	}
	Ok(())
}

fn depth(value: &Value, level: usize) -> Result<(), Error> {
	if level > MAX_DEPTH {
		return Err(Error::TooLarge);
	}
	match value {
		Value::Array(values) => values.iter().try_for_each(|value| depth(value, level + 1)),
		Value::Object(values) => {
			values
				.values()
				.try_for_each(|value| depth(value, level + 1))
		}
		_ => Ok(()),
	}
}

struct Limited {
	bytes: Vec<u8>,
	limit: usize,
}

impl Write for Limited {
	fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
		if bytes.len() > self.limit.saturating_sub(self.bytes.len()) {
			return Err(io::Error::other("settings limit exceeded"));
		}
		self.bytes.extend_from_slice(bytes);
		Ok(bytes.len())
	}

	fn flush(&mut self) -> io::Result<()> {
		Ok(())
	}
}

fn json(value: &impl Serialize, limit: usize) -> Result<Vec<u8>, Error> {
	let mut output = Limited {
		bytes: Vec::new(),
		limit,
	};
	serde_json::to_writer(&mut output, value).map_err(|_| Error::TooLarge)?;
	Ok(output.bytes)
}

pub(crate) fn value_json(value: &Value) -> Result<Vec<u8>, Error> {
	depth(value, 0)?;
	json(value, MAX_VALUE_BYTES)
}

pub(crate) fn read_value(bytes: &[u8]) -> Result<Value, Error> {
	if bytes.len() > MAX_VALUE_BYTES {
		return Err(Error::TooLarge);
	}
	let value = serde_json::from_slice(bytes).map_err(|_| Error::InvalidArgument)?;
	depth(&value, 0)?;
	Ok(value)
}

pub(crate) fn encode(entries: &Entries) -> Result<Vec<u8>, Error> {
	if entries.len() > MAX_ENTRIES {
		return Err(Error::TooLarge);
	}
	for (name, value) in entries {
		key(name)?;
		value_json(value)?;
	}
	#[derive(Serialize)]
	struct BorrowedDocument<'a> {
		version: u32,
		entries: &'a Entries,
	}
	json(
		&BorrowedDocument {
			version: 1,
			entries,
		},
		MAX_STORE_BYTES,
	)
}

pub(crate) fn decode(bytes: &[u8]) -> Result<Entries, Error> {
	if bytes.len() > MAX_STORE_BYTES {
		return Err(Error::TooLarge);
	}
	let document: Document = serde_json::from_slice(bytes).map_err(|_| Error::InvalidArgument)?;
	if document.version != 1 {
		return Err(Error::InvalidArgument);
	}
	encode(&document.entries)?;
	Ok(document.entries)
}

#[cfg(any(not(target_arch = "wasm32"), not(feature = "guest"), test))]
pub(crate) fn candidate(
	entries: &Entries,
	changes: &[Change],
) -> Result<(Entries, Vec<u8>), Error> {
	if changes.len() > MAX_CHANGES {
		return Err(Error::TooLarge);
	}
	let mut next = entries.clone();
	let mut size = encode(entries)?.len();
	for change in changes {
		match change {
			Change::Set(name, value) => {
				key(name)?;
				let value_size = value_json(value)?.len();
				let name_size = json(name, MAX_KEY_BYTES * 6 + 2)?.len();
				let replacement_size = match next.get(name) {
					Some(old) => size - value_json(old)?.len() + value_size,
					None => size + name_size + 1 + value_size + usize::from(!next.is_empty()),
				};
				if replacement_size > MAX_STORE_BYTES {
					return Err(Error::TooLarge);
				}
				size = replacement_size;
				next.insert(name.clone(), value.clone());
			}
			Change::Remove(name) => {
				key(name)?;
				if let Some(old) = next.remove(name) {
					size -= json(name, MAX_KEY_BYTES * 6 + 2)?.len()
						+ 1
						+ value_json(&old)?.len()
						+ usize::from(!next.is_empty());
				}
			}
			Change::Clear => {
				next.clear();
				size = br#"{"version":1,"entries":{}}"#.len();
			}
		}
		// Do not permit transient intermediate states to exceed the key budget.
		if next.len() > MAX_ENTRIES {
			return Err(Error::TooLarge);
		}
	}
	let bytes = encode(&next)?;
	Ok((next, bytes))
}

/// Encodes a bounded binary mutation envelope for the [`APPLY`] method.
pub fn encode_changes(changes: &[Change], save: Save) -> Result<Bytes, Error> {
	if changes.len() > MAX_CHANGES {
		return Err(Error::TooLarge);
	}
	let mut output = Encoder::with_capacity(8);
	output.u32(u32::from(save == Save::Commit));
	output.u32(u32::try_from(changes.len()).map_err(|_| Error::TooLarge)?);
	let mut size = 8;
	for change in changes {
		match change {
			Change::Set(name, value) => {
				key(name)?;
				let value = value_json(value)?;
				size += 12 + name.len() + value.len();
				if size > MAX_STORE_BYTES {
					return Err(Error::TooLarge);
				}
				output.u32(0);
				output.bytes(name.as_bytes())?;
				output.bytes(&value)?;
			}
			Change::Remove(name) => {
				key(name)?;
				size += 8 + name.len();
				if size > MAX_STORE_BYTES {
					return Err(Error::TooLarge);
				}
				output.u32(1);
				output.bytes(name.as_bytes())?;
			}
			Change::Clear => {
				size += 4;
				if size > MAX_STORE_BYTES {
					return Err(Error::TooLarge);
				}
				output.u32(2);
			}
		}
	}
	let bytes = output.finish();
	if bytes.len() > MAX_STORE_BYTES {
		return Err(Error::TooLarge);
	}
	Ok(bytes)
}

/// Validates the entire binary mutation envelope, including trailing bytes.
pub fn decode_changes(bytes: &[u8]) -> Result<(Vec<Change>, Save), Error> {
	if bytes.len() > MAX_STORE_BYTES {
		return Err(Error::TooLarge);
	}
	let mut input = Decoder::new(bytes);
	let save = match input.u32()? {
		0 => Save::Deferred,
		1 => Save::Commit,
		_ => return Err(Error::InvalidArgument),
	};
	let count = input.u32()? as usize;
	if count > MAX_CHANGES {
		return Err(Error::TooLarge);
	}
	let mut changes = Vec::with_capacity(count);
	for _ in 0..count {
		changes.push(match input.u32()? {
			0 => {
				let name = input.string()?;
				key(name)?;
				Change::Set(name.into(), read_value(input.bytes()?)?)
			}
			1 => {
				let name = input.string()?;
				key(name)?;
				Change::Remove(name.into())
			}
			2 => Change::Clear,
			_ => return Err(Error::InvalidArgument),
		});
	}
	input.finish()?;
	Ok((changes, save))
}

#[cfg(not(target_arch = "wasm32"))]
mod native;
#[cfg(not(target_arch = "wasm32"))]
pub use native::Store;
#[cfg(not(target_arch = "wasm32"))]
pub use native::from_app_dirs;

/// Origin-local browser settings.
#[cfg(all(target_arch = "wasm32", target_os = "unknown", not(feature = "guest")))]
pub mod web;

/// Cached native guest client.
#[cfg(all(target_arch = "wasm32", feature = "guest"))]
pub mod guest;

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn binary_batch_roundtrips_and_rejects_malformed_input() {
		let changes = [
			Change::Set("theme".into(), Value::Null),
			Change::Remove("old".into()),
			Change::Clear,
		];
		let bytes = encode_changes(&changes, Save::Commit).unwrap();
		assert_eq!(decode_changes(&bytes), Ok((changes.to_vec(), Save::Commit)));
		for truncated in 0..bytes.len() {
			assert!(decode_changes(&bytes[..truncated]).is_err());
		}
		let mut trailing = bytes.to_vec();
		trailing.push(0);
		assert_eq!(decode_changes(&trailing), Err(Error::InvalidArgument));
		let mut invalid = bytes.to_vec();
		invalid[0] = 2;
		assert_eq!(decode_changes(&invalid), Err(Error::InvalidArgument));
	}

	#[test]
	fn limits_and_failed_batches_leave_original_unchanged() {
		let entries = BTreeMap::from([("theme".into(), Value::from("dark"))]);
		let original = entries.clone();
		let changes = [Change::Clear, Change::Set(String::new(), Value::Null)];
		assert_eq!(candidate(&entries, &changes), Err(Error::InvalidArgument));
		assert_eq!(entries, original);
		assert_eq!(key(&"x".repeat(MAX_KEY_BYTES + 1)), Err(Error::TooLarge));
		assert_eq!(
			value_json(&Value::from("x".repeat(MAX_VALUE_BYTES))),
			Err(Error::TooLarge)
		);
		let mut deep = Value::Null;
		for _ in 0..=MAX_DEPTH {
			deep = Value::Array(vec![deep]);
		}
		assert_eq!(value_json(&deep), Err(Error::TooLarge));
		let many = vec![Change::Clear; MAX_CHANGES + 1];
		assert_eq!(candidate(&entries, &many), Err(Error::TooLarge));
		assert_eq!(read_value(b"{broken"), Err(Error::InvalidArgument));
		assert_eq!(
			decode(br#"{"version":2,"entries":{}}"#),
			Err(Error::InvalidArgument)
		);
		assert_eq!(
			decode(br#"{"version":1,"entries":{},"extra":0}"#),
			Err(Error::InvalidArgument)
		);
		assert_eq!(
			decode(br#"{"version":1,"entries":{"duplicate":1,"duplicate":2}}"#),
			Err(Error::InvalidArgument)
		);
		let oversized: Entries = (0..=MAX_ENTRIES)
			.map(|index| (index.to_string(), Value::Null))
			.collect();
		assert_eq!(encode(&oversized), Err(Error::TooLarge));
		let changes: Vec<_> = (0..9)
			.map(|index| {
				Change::Set(
					index.to_string(),
					Value::from("x".repeat(MAX_VALUE_BYTES - 2)),
				)
			})
			.collect();
		assert_eq!(candidate(&Entries::new(), &changes), Err(Error::TooLarge));
		assert_eq!(encode_changes(&changes, Save::Commit), Err(Error::TooLarge));
	}
}
