//! Values as they cross into and out of the database, independent of driver.

use std::fmt;

use chrono::NaiveDate;
use leptoskit_db_schema::ColumnType;
use leptoskit_db_schema::TableName;
use uuid::Uuid;

use crate::Id;
use crate::Json;
use crate::Table;
use crate::Timestamp;

/// One column's value. Each driver maps these to its own types: a UUID is
/// `uuid` on Postgres and 16 bytes on SQLite, a timestamp `timestamptz` or
/// RFC 3339 text.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
	/// SQL `NULL`.
	Null,
	/// A boolean.
	Bool(bool),
	/// Any integer, widened to 64 bits.
	Int(i64),
	/// Any float, widened to 64 bits.
	Float(f64),
	/// Text.
	Text(String),
	/// Bytes.
	Bytes(Vec<u8>),
	/// A UUID.
	Uuid(Uuid),
	/// An instant.
	Timestamp(Timestamp),
	/// A calendar date.
	Date(NaiveDate),
	/// A JSON document.
	Json(serde_json::Value),
}

impl Value {
	/// The kind of value, for error messages.
	pub fn kind(&self) -> &'static str {
		match self {
			Self::Null => "null",
			Self::Bool(_) => "bool",
			Self::Int(_) => "integer",
			Self::Float(_) => "float",
			Self::Text(_) => "text",
			Self::Bytes(_) => "bytes",
			Self::Uuid(_) => "uuid",
			Self::Timestamp(_) => "timestamp",
			Self::Date(_) => "date",
			Self::Json(_) => "json",
		}
	}
}

/// A Rust type that can be a column.
///
/// Implemented for the primitives, `String`, `Vec<u8>`, `Uuid`, [`Id`],
/// [`Timestamp`], `NaiveDate`, [`Json`], `Option` of any of these, and every
/// `#[derive(db::Enum)]` enum. Implement it for your own types to store them
/// directly, rather than converting in getters and setters.
pub trait Column: Sized {
	/// What the column stores.
	fn column_type() -> ColumnType;

	/// Whether the column accepts `NULL`.
	fn nullable() -> bool {
		false
	}

	/// The table and columns a column of this type references, for `Id<T>`.
	fn references() -> Option<Reference> {
		None
	}

	/// The only values the column may hold, for enums.
	fn allowed_values() -> Option<&'static [&'static str]> {
		None
	}

	/// This value, ready to bind.
	fn to_value(&self) -> Result<Value, EncodeError>;

	/// The Rust value for a database value.
	fn from_value(value: Value) -> Result<Self, DecodeError>;
}

/// What a foreign key column points at.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reference {
	/// The referenced table.
	pub table: TableName,
	/// Its key columns.
	pub columns: Vec<String>,
}

/// A value that can't be stored.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct EncodeError(String);

impl EncodeError {
	/// An error with this message.
	pub fn new(message: impl fmt::Display) -> Self {
		Self(message.to_string())
	}
}

/// A database value that isn't what the Rust type expects.
#[derive(Debug, thiserror::Error)]
#[error("{}{message}", location.as_deref().map(|location| format!("{location}: ")).unwrap_or_default())]
pub struct DecodeError {
	location: Option<String>,
	message: String,
}

impl DecodeError {
	/// The value was a different kind than expected.
	pub fn unexpected(expected: &str, found: &Value) -> Self {
		Self::custom(format!("expected {expected}, found {}", found.kind()))
	}

	/// An error with this message.
	pub fn custom(message: impl fmt::Display) -> Self {
		Self {
			location: None,
			message: message.to_string(),
		}
	}

	/// The same error, located at a table's column.
	#[must_use]
	pub fn at(mut self, table: &str, column: &str) -> Self {
		self.location
			.get_or_insert_with(|| format!("{table}.{column}"));
		self
	}
}

impl<T: Column> Column for Option<T> {
	fn column_type() -> ColumnType {
		T::column_type()
	}

	fn nullable() -> bool {
		true
	}

	fn references() -> Option<Reference> {
		T::references()
	}

	fn allowed_values() -> Option<&'static [&'static str]> {
		T::allowed_values()
	}

	fn to_value(&self) -> Result<Value, EncodeError> {
		self.as_ref().map_or(Ok(Value::Null), T::to_value)
	}

	fn from_value(value: Value) -> Result<Self, DecodeError> {
		match value {
			Value::Null => Ok(None),
			value => T::from_value(value).map(Some),
		}
	}
}

impl Column for bool {
	fn column_type() -> ColumnType {
		ColumnType::Bool
	}

	fn to_value(&self) -> Result<Value, EncodeError> {
		Ok(Value::Bool(*self))
	}

	fn from_value(value: Value) -> Result<Self, DecodeError> {
		match value {
			Value::Bool(value) => Ok(value),
			// SQLite stores booleans as 0 or 1.
			Value::Int(value) => Ok(value != 0),
			other => Err(DecodeError::unexpected("bool", &other)),
		}
	}
}

/// Integers convert through `i64`, checked both ways.
macro_rules! integers {
	($($ty:ty => $column:ident),* $(,)?) => {
		$(impl Column for $ty {
			fn column_type() -> ColumnType {
				ColumnType::$column
			}

			fn to_value(&self) -> Result<Value, EncodeError> {
				i64::try_from(*self)
					.map(Value::Int)
					.map_err(|_| EncodeError::new(format!("{self} doesn't fit a 64-bit signed column")))
			}

			fn from_value(value: Value) -> Result<Self, DecodeError> {
				match value {
					Value::Int(value) => <$ty>::try_from(value)
						.map_err(|_| DecodeError::custom(format!("{value} doesn't fit {}", stringify!($ty)))),
					other => Err(DecodeError::unexpected("integer", &other)),
				}
			}
		})*
	};
}

integers! {
	i16 => Int16,
	i32 => Int32,
	i64 => Int64,
	u8 => Int16,
	u16 => Int32,
	u32 => Int64,
	u64 => Int64,
}

impl Column for f32 {
	fn column_type() -> ColumnType {
		ColumnType::Float32
	}

	fn to_value(&self) -> Result<Value, EncodeError> {
		Ok(Value::Float(f64::from(*self)))
	}

	fn from_value(value: Value) -> Result<Self, DecodeError> {
		f64::from_value(value).map(|value| value as f32)
	}
}

impl Column for f64 {
	fn column_type() -> ColumnType {
		ColumnType::Float64
	}

	fn to_value(&self) -> Result<Value, EncodeError> {
		Ok(Value::Float(*self))
	}

	fn from_value(value: Value) -> Result<Self, DecodeError> {
		match value {
			Value::Float(value) => Ok(value),
			Value::Int(value) => Ok(value as f64),
			other => Err(DecodeError::unexpected("float", &other)),
		}
	}
}

impl Column for String {
	fn column_type() -> ColumnType {
		ColumnType::TEXT
	}

	fn to_value(&self) -> Result<Value, EncodeError> {
		Ok(Value::Text(self.clone()))
	}

	fn from_value(value: Value) -> Result<Self, DecodeError> {
		match value {
			Value::Text(value) => Ok(value),
			other => Err(DecodeError::unexpected("text", &other)),
		}
	}
}

impl Column for Vec<u8> {
	fn column_type() -> ColumnType {
		ColumnType::Bytes
	}

	fn to_value(&self) -> Result<Value, EncodeError> {
		Ok(Value::Bytes(self.clone()))
	}

	fn from_value(value: Value) -> Result<Self, DecodeError> {
		match value {
			Value::Bytes(value) => Ok(value),
			other => Err(DecodeError::unexpected("bytes", &other)),
		}
	}
}

impl Column for Uuid {
	fn column_type() -> ColumnType {
		ColumnType::Uuid
	}

	fn to_value(&self) -> Result<Value, EncodeError> {
		Ok(Value::Uuid(*self))
	}

	fn from_value(value: Value) -> Result<Self, DecodeError> {
		match value {
			Value::Uuid(value) => Ok(value),
			// SQLite stores UUIDs as 16 bytes.
			Value::Bytes(bytes) => Uuid::from_slice(&bytes).map_err(DecodeError::custom),
			other => Err(DecodeError::unexpected("uuid", &other)),
		}
	}
}

impl<T: Table> Column for Id<T> {
	fn column_type() -> ColumnType {
		ColumnType::Uuid
	}

	fn references() -> Option<Reference> {
		Some(Reference {
			table: T::table_name(),
			columns: T::KEY.iter().map(ToString::to_string).collect(),
		})
	}

	fn to_value(&self) -> Result<Value, EncodeError> {
		Ok(Value::Uuid(self.uuid()))
	}

	fn from_value(value: Value) -> Result<Self, DecodeError> {
		Uuid::from_value(value).map(Self::from_uuid)
	}
}

impl Column for Timestamp {
	fn column_type() -> ColumnType {
		ColumnType::Timestamp
	}

	fn to_value(&self) -> Result<Value, EncodeError> {
		Ok(Value::Timestamp(*self))
	}

	fn from_value(value: Value) -> Result<Self, DecodeError> {
		match value {
			Value::Timestamp(value) => Ok(value),
			// SQLite stores timestamps as RFC 3339 text.
			Value::Text(text) => {
				chrono::DateTime::parse_from_rfc3339(&text)
					.map(|value| value.to_utc())
					.map_err(DecodeError::custom)
			}
			other => Err(DecodeError::unexpected("timestamp", &other)),
		}
	}
}

impl Column for NaiveDate {
	fn column_type() -> ColumnType {
		ColumnType::Date
	}

	fn to_value(&self) -> Result<Value, EncodeError> {
		Ok(Value::Date(*self))
	}

	fn from_value(value: Value) -> Result<Self, DecodeError> {
		match value {
			Value::Date(value) => Ok(value),
			Value::Text(text) => text.parse().map_err(DecodeError::custom),
			other => Err(DecodeError::unexpected("date", &other)),
		}
	}
}

impl<T: serde::Serialize + serde::de::DeserializeOwned> Column for Json<T> {
	fn column_type() -> ColumnType {
		ColumnType::Json
	}

	fn to_value(&self) -> Result<Value, EncodeError> {
		serde_json::to_value(&self.0)
			.map(Value::Json)
			.map_err(EncodeError::new)
	}

	fn from_value(value: Value) -> Result<Self, DecodeError> {
		let json = match value {
			Value::Json(json) => json,
			// SQLite stores JSON as text.
			Value::Text(text) => serde_json::from_str(&text).map_err(DecodeError::custom)?,
			other => return Err(DecodeError::unexpected("json", &other)),
		};

		serde_json::from_value(json)
			.map(Json)
			.map_err(DecodeError::custom)
	}
}
