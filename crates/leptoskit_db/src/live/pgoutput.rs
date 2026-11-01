//! Postgres's `pgoutput` logical decoding messages, protocol version 1, as
//! `pg_logical_slot_peek_binary_changes` returns them: what replication
//! capture reads the write-ahead log through.
//!
//! Every value arrives as its type's text output, and is turned into the
//! JSON `to_jsonb` would have made of it, so a change captured this way
//! reads exactly like one a capture trigger wrote.

use crate::DbError;
use crate::DecodeError;
use crate::Result;

/// One decoded message.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Message {
	/// A transaction starts.
	Begin {
		/// Its transaction ID.
		xid: u32,
	},
	/// The transaction committed.
	Commit {
		/// Where its commit record starts.
		commit_lsn: u64,
		/// Where its commit record ends: decoding past it skips the
		/// transaction.
		end_lsn: u64,
		/// When it committed, in microseconds since 2000-01-01 UTC.
		timestamp: i64,
	},
	/// A table's columns, sent before the first change to it.
	Relation(Relation),
	/// A row was inserted.
	Insert {
		/// The table's relation ID.
		relation: u32,
		/// The row.
		new: Vec<Datum>,
	},
	/// A row was updated.
	Update {
		/// The table's relation ID.
		relation: u32,
		/// The row before, whole under `REPLICA IDENTITY FULL`.
		old: Option<Vec<Datum>>,
		/// The row after.
		new: Vec<Datum>,
	},
	/// A row was deleted.
	Delete {
		/// The table's relation ID.
		relation: u32,
		/// The row before.
		old: Vec<Datum>,
	},
	/// Tables were emptied.
	Truncate {
		/// Their relation IDs.
		relations: Vec<u32>,
	},
	/// A logical message the transaction emitted.
	Logical {
		/// Whether it belongs to the transaction rather than standing alone.
		transactional: bool,
		/// What it's about.
		prefix: String,
		/// What it says.
		content: Vec<u8>,
	},
	/// Something capture doesn't need: a type, or where a change came from.
	Other,
}

/// A table as a `Relation` message describes it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Relation {
	pub(crate) id: u32,
	pub(crate) namespace: String,
	pub(crate) name: String,
	pub(crate) columns: Vec<RelationColumn>,
}

/// One column of a [`Relation`].
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RelationColumn {
	pub(crate) name: String,
	pub(crate) type_oid: u32,
}

/// One column's value in a row.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Datum {
	Null,
	/// A large value the update didn't change, which isn't sent again.
	Unchanged,
	/// The value as its type's text output.
	Text(String),
}

/// Decodes one message.
pub(crate) fn decode(bytes: &[u8]) -> Result<Message> {
	let mut reader = Reader { bytes, at: 0 };
	let kind = reader.u8()?;

	Ok(match kind {
		b'B' => {
			let _final_lsn = reader.u64()?;
			let _timestamp = reader.i64()?;
			Message::Begin { xid: reader.u32()? }
		}
		b'C' => {
			let _flags = reader.u8()?;
			Message::Commit {
				commit_lsn: reader.u64()?,
				end_lsn: reader.u64()?,
				timestamp: reader.i64()?,
			}
		}
		b'R' => Message::Relation(relation(&mut reader)?),
		b'I' => {
			let relation = reader.u32()?;
			reader.expect(b'N')?;
			Message::Insert {
				relation,
				new: tuple(&mut reader)?,
			}
		}
		b'U' => {
			let relation = reader.u32()?;
			let mut old = None;
			let mut marker = reader.u8()?;

			if marker == b'K' || marker == b'O' {
				old = Some(tuple(&mut reader)?);
				marker = reader.u8()?;
			}

			if marker != b'N' {
				return Err(malformed("an update without its new row"));
			}

			Message::Update {
				relation,
				old,
				new: tuple(&mut reader)?,
			}
		}
		b'D' => {
			let relation = reader.u32()?;

			match reader.u8()? {
				b'K' | b'O' => {}
				_ => return Err(malformed("a delete without its old row")),
			}

			Message::Delete {
				relation,
				old: tuple(&mut reader)?,
			}
		}
		b'T' => {
			let count = reader.u32()?;
			let _options = reader.u8()?;
			let relations = (0..count)
				.map(|_| reader.u32())
				.collect::<Result<Vec<_>>>()?;
			Message::Truncate { relations }
		}
		b'M' => {
			let flags = reader.u8()?;
			let _lsn = reader.u64()?;
			let prefix = reader.string()?;
			let length = reader.u32()? as usize;
			Message::Logical {
				transactional: flags & 1 == 1,
				prefix,
				content: reader.take(length)?.to_vec(),
			}
		}
		b'Y' | b'O' => Message::Other,
		other => {
			return Err(malformed(&format!(
				"an unknown message `{}`",
				char::from(other)
			)));
		}
	})
}

fn relation(reader: &mut Reader<'_>) -> Result<Relation> {
	let id = reader.u32()?;
	let namespace = reader.string()?;
	let name = reader.string()?;
	let _identity = reader.u8()?;
	let count = reader.u16()?;
	let columns = (0..count)
		.map(|_| {
			let _flags = reader.u8()?;
			let name = reader.string()?;
			let type_oid = reader.u32()?;
			let _modifier = reader.i32()?;
			Ok(RelationColumn { name, type_oid })
		})
		.collect::<Result<Vec<_>>>()?;

	Ok(Relation {
		id,
		namespace,
		name,
		columns,
	})
}

fn tuple(reader: &mut Reader<'_>) -> Result<Vec<Datum>> {
	let count = reader.u16()?;

	(0..count)
		.map(|_| {
			match reader.u8()? {
				b'n' => Ok(Datum::Null),
				b'u' => Ok(Datum::Unchanged),
				b't' => {
					let length = reader.u32()? as usize;
					let text = std::str::from_utf8(reader.take(length)?)
						.map_err(|_| malformed("a value that isn't UTF-8"))?;
					Ok(Datum::Text(text.to_string()))
				}
				_ => Err(malformed("a value in an unknown format")),
			}
		})
		.collect()
}

fn malformed(what: &str) -> DbError {
	DbError::Decode(DecodeError::custom(format!(
		"replication capture read {what} from pgoutput"
	)))
}

/// Reads big-endian integers and C strings from a message.
struct Reader<'a> {
	bytes: &'a [u8],
	at: usize,
}

impl<'a> Reader<'a> {
	fn take(&mut self, length: usize) -> Result<&'a [u8]> {
		let end = self
			.at
			.checked_add(length)
			.filter(|end| *end <= self.bytes.len())
			.ok_or_else(|| malformed("a message cut short"))?;
		let taken = &self.bytes[self.at..end];
		self.at = end;
		Ok(taken)
	}

	fn array<const N: usize>(&mut self) -> Result<[u8; N]> {
		let mut array = [0; N];
		array.copy_from_slice(self.take(N)?);
		Ok(array)
	}

	fn u8(&mut self) -> Result<u8> {
		Ok(self.array::<1>()?[0])
	}

	fn expect(&mut self, wanted: u8) -> Result<()> {
		if self.u8()? == wanted {
			Ok(())
		} else {
			Err(malformed(&format!(
				"a message without its `{}` part",
				char::from(wanted)
			)))
		}
	}

	fn u16(&mut self) -> Result<u16> {
		Ok(u16::from_be_bytes(self.array()?))
	}

	fn u32(&mut self) -> Result<u32> {
		Ok(u32::from_be_bytes(self.array()?))
	}

	fn i32(&mut self) -> Result<i32> {
		Ok(i32::from_be_bytes(self.array()?))
	}

	fn u64(&mut self) -> Result<u64> {
		Ok(u64::from_be_bytes(self.array()?))
	}

	fn i64(&mut self) -> Result<i64> {
		Ok(i64::from_be_bytes(self.array()?))
	}

	fn string(&mut self) -> Result<String> {
		let rest = &self.bytes[self.at..];
		let end = rest
			.iter()
			.position(|byte| *byte == 0)
			.ok_or_else(|| malformed("a name without its end"))?;
		let text = std::str::from_utf8(&rest[..end])
			.map_err(|_| malformed("a name that isn't UTF-8"))?
			.to_string();
		self.at += end + 1;
		Ok(text)
	}
}

/// The microseconds between the Unix epoch and Postgres's, 2000-01-01.
const POSTGRES_EPOCH_MICROS: i64 = 946_684_800_000_000;

/// A commit's timestamp, from microseconds since Postgres's epoch.
pub(crate) fn timestamp(micros: i64) -> crate::Timestamp {
	chrono::DateTime::from_timestamp_micros(micros.saturating_add(POSTGRES_EPOCH_MICROS))
		.unwrap_or_default()
}

/// The built-in types whose text output `to_jsonb` turns into something
/// other than a string, by OID.
mod oid {
	pub(super) const BOOL: u32 = 16;
	pub(super) const INT8: u32 = 20;
	pub(super) const INT2: u32 = 21;
	pub(super) const INT4: u32 = 23;
	pub(super) const OID: u32 = 26;
	pub(super) const JSON: u32 = 114;
	pub(super) const FLOAT4: u32 = 700;
	pub(super) const FLOAT8: u32 = 701;
	pub(super) const TIMESTAMP: u32 = 1114;
	pub(super) const TIMESTAMPTZ: u32 = 1184;
	pub(super) const NUMERIC: u32 = 1700;
	pub(super) const JSONB: u32 = 3802;
}

/// The element type of a built-in array type, by the array's OID.
fn element_of(array: u32) -> Option<u32> {
	Some(match array {
		1000 => oid::BOOL,
		1005 => oid::INT2,
		1007 => oid::INT4,
		1016 => oid::INT8,
		1021 => oid::FLOAT4,
		1022 => oid::FLOAT8,
		1231 => oid::NUMERIC,
		1115 => oid::TIMESTAMP,
		1185 => oid::TIMESTAMPTZ,
		199 => oid::JSON,
		3807 => oid::JSONB,
		// Text, names, UUIDs, dates, bytes, and the like: strings.
		1009 | 1015 | 1014 | 1003 | 2951 | 1182 | 1183 | 1001 | 1002 | 1028 => 25,
		_ => return None,
	})
}

/// A value of the type `type_oid`, from its text output, as `to_jsonb`
/// writes it: numbers and booleans as JSON's own, JSON as itself, arrays as
/// arrays, timestamps in ISO 8601 with a `T`, and everything else, such as
/// UUIDs, dates, bytes (`\x…`), and extension types, as the text itself.
pub(crate) fn json(type_oid: u32, text: &str) -> serde_json::Value {
	use serde_json::Value as Json;

	if let Some(element) = element_of(type_oid) {
		return array(text, element).unwrap_or_else(|| Json::String(text.to_string()));
	}

	match type_oid {
		oid::BOOL => Json::Bool(text == "t"),
		oid::INT2 | oid::INT4 | oid::INT8 | oid::OID => {
			text.parse::<i64>()
				.map_or_else(|_| Json::String(text.to_string()), Json::from)
		}
		// `NaN` and the infinities, which JSON has no numbers for, stay text.
		oid::FLOAT4 | oid::FLOAT8 | oid::NUMERIC => {
			serde_json::from_str::<serde_json::Number>(text)
				.map_or_else(|_| Json::String(text.to_string()), Json::Number)
		}
		oid::JSON | oid::JSONB => {
			serde_json::from_str(text).unwrap_or_else(|_| Json::String(text.to_string()))
		}
		oid::TIMESTAMP | oid::TIMESTAMPTZ => Json::String(iso_timestamp(text)),
		_ => Json::String(text.to_string()),
	}
}

/// A timestamp as Postgres's ISO `DateStyle` writes it, `2026-11-01
/// 12:00:00.5+00`, as `to_jsonb` does: `2026-11-01T12:00:00.5+00:00`.
/// `infinity` and its negative stay as they are.
fn iso_timestamp(text: &str) -> String {
	let Some((date, time)) = text.split_once(' ') else {
		return text.to_string();
	};
	let offset_at = time.rfind(['+', '-']);

	match offset_at {
		Some(at) if time.len() - at == 3 => format!("{date}T{time}:00"),
		_ => format!("{date}T{time}"),
	}
}

/// A one-dimensional or nested array literal, `{1,NULL,"a b"}`, with each
/// element read as `element`. `None` when it isn't one.
fn array(text: &str, element: u32) -> Option<serde_json::Value> {
	let mut chars = text.chars().peekable();

	// A lower bound other than 1 prefixes the literal: `[0:1]={…}`.
	if chars.peek() == Some(&'[') {
		for character in chars.by_ref() {
			if character == '=' {
				break;
			}
		}
	}

	let value = array_items(&mut chars, element)?;
	chars.next().is_none().then_some(value)
}

fn array_items(
	chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
	element: u32,
) -> Option<serde_json::Value> {
	if chars.next()? != '{' {
		return None;
	}

	let mut items = Vec::new();

	if chars.peek() == Some(&'}') {
		chars.next();
		return Some(serde_json::Value::Array(items));
	}

	loop {
		let item = match chars.peek()? {
			'{' => array_items(chars, element)?,
			'"' => {
				chars.next();
				let mut text = String::new();

				loop {
					match chars.next()? {
						'\\' => text.push(chars.next()?),
						'"' => break,
						character => text.push(character),
					}
				}

				json(element, &text)
			}
			_ => {
				let mut text = String::new();

				while let Some(&character) = chars.peek() {
					if character == ',' || character == '}' {
						break;
					}

					text.push(character);
					chars.next();
				}

				if text == "NULL" {
					serde_json::Value::Null
				} else {
					json(element, &text)
				}
			}
		};
		items.push(item);

		match chars.next()? {
			',' => {}
			'}' => return Some(serde_json::Value::Array(items)),
			_ => return None,
		}
	}
}

#[cfg(test)]
mod tests {
	use serde_json::json as j;

	use super::*;

	#[test]
	fn values_read_as_to_jsonb_writes_them() {
		assert_eq!(json(oid::BOOL, "t"), j!(true));
		assert_eq!(json(oid::BOOL, "f"), j!(false));
		assert_eq!(json(oid::INT8, "-42"), j!(-42));
		assert_eq!(json(oid::FLOAT8, "1.5"), j!(1.5));
		assert_eq!(json(oid::FLOAT8, "NaN"), j!("NaN"));
		assert_eq!(json(oid::FLOAT4, "-Infinity"), j!("-Infinity"));
		assert_eq!(json(oid::JSONB, "{\"a\": [1]}"), j!({ "a": [1] }));
		assert_eq!(
			json(oid::TIMESTAMPTZ, "2026-11-01 12:00:00.5+00"),
			j!("2026-11-01T12:00:00.5+00:00")
		);
		assert_eq!(
			json(oid::TIMESTAMPTZ, "2026-11-01 12:00:00+05:30"),
			j!("2026-11-01T12:00:00+05:30")
		);
		assert_eq!(
			json(oid::TIMESTAMP, "2026-11-01 12:00:00"),
			j!("2026-11-01T12:00:00")
		);
		assert_eq!(json(oid::TIMESTAMPTZ, "infinity"), j!("infinity"));
		assert_eq!(json(2950, "0192-…"), j!("0192-…"));
		assert_eq!(json(17, "\\x01ab"), j!("\\x01ab"));
		assert_eq!(json(1007, "{1,NULL,3}"), j!([1, null, 3]));
		assert_eq!(
			json(1009, "{\"a b\",c,\"q\\\"d\",\"NULL\"}"),
			j!(["a b", "c", "q\"d", "NULL"])
		);
		assert_eq!(json(1007, "{{1,2},{3,4}}"), j!([[1, 2], [3, 4]]));
		assert_eq!(json(1007, "[0:1]={5,6}"), j!([5, 6]));
		assert_eq!(json(1007, "{}"), j!([]));
		assert_eq!(json(1007, "{1,2"), j!("{1,2"));
	}

	#[test]
	fn messages_decode() {
		let mut relation = vec![b'R'];
		relation.extend(16_384_u32.to_be_bytes());
		relation.extend(b"public\0posts\0");
		relation.push(b'f');
		relation.extend(2_u16.to_be_bytes());

		for (name, oid) in [("id", 23_u32), ("title", 25)] {
			relation.push(1);
			relation.extend(name.as_bytes());
			relation.push(0);
			relation.extend(oid.to_be_bytes());
			relation.extend((-1_i32).to_be_bytes());
		}

		assert_eq!(
			decode(&relation).unwrap(),
			Message::Relation(Relation {
				id: 16_384,
				namespace: "public".into(),
				name: "posts".into(),
				columns: vec![
					RelationColumn {
						name: "id".into(),
						type_oid: 23
					},
					RelationColumn {
						name: "title".into(),
						type_oid: 25
					},
				],
			})
		);

		let mut update = vec![b'U'];
		update.extend(16_384_u32.to_be_bytes());
		update.push(b'O');
		update.extend(2_u16.to_be_bytes());
		update.push(b't');
		update.extend(1_u32.to_be_bytes());
		update.push(b'7');
		update.push(b'n');
		update.push(b'N');
		update.extend(2_u16.to_be_bytes());
		update.push(b't');
		update.extend(1_u32.to_be_bytes());
		update.push(b'7');
		update.push(b'u');
		assert_eq!(
			decode(&update).unwrap(),
			Message::Update {
				relation: 16_384,
				old: Some(vec![Datum::Text("7".into()), Datum::Null]),
				new: vec![Datum::Text("7".into()), Datum::Unchanged],
			}
		);

		let mut commit = vec![b'C', 0];
		commit.extend(10_u64.to_be_bytes());
		commit.extend(20_u64.to_be_bytes());
		commit.extend(0_i64.to_be_bytes());
		assert_eq!(
			decode(&commit).unwrap(),
			Message::Commit {
				commit_lsn: 10,
				end_lsn: 20,
				timestamp: 0
			}
		);
		assert_eq!(timestamp(0).to_rfc3339(), "2000-01-01T00:00:00+00:00");

		assert!(decode(&[b'U', 0, 0]).is_err());
		assert!(decode(b"Z").is_err());
		assert!(decode(&[]).is_err());
	}
}
