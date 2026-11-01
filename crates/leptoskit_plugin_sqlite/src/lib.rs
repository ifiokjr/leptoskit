//! Application-private `SQLite` with typed bindings and bounded binary
//! messages.
//!
//! Native calls run on the host's Rust worker. Each statement or explicit batch
//! is atomic. Browser builds expose the types, but have no `SQLite`/OPFS
//! backend.

use leptoskit_bridge::Bytes;
use leptoskit_bridge::Error;
use leptoskit_bridge::MethodId;
use leptoskit_bridge::codec::Decoder;
use leptoskit_bridge::codec::Encoder;

/// Execute one parameterized statement, returning affected rows and insert ID.
pub const EXECUTE: MethodId = MethodId(0);
/// Query one read-only statement, returning column names and typed rows.
pub const QUERY: MethodId = MethodId(1);
/// Execute a non-empty batch atomically, rolling back on any error.
pub const TRANSACTION: MethodId = MethodId(2);
/// Close the connection; the next operation reopens the same database.
pub const CLOSE: MethodId = MethodId(3);
/// Maximum encoded request or response size.
pub const MAX_BYTES: usize = 1024 * 1024;
/// Maximum UTF-8 SQL length per statement.
pub const MAX_SQL_BYTES: usize = 64 * 1024;
/// Maximum returned rows. Larger results must use SQL pagination.
pub const MAX_ROWS: usize = 1024;
/// Maximum columns per row.
pub const MAX_COLUMNS: usize = 128;
/// Maximum bound positional parameters per statement.
pub const MAX_PARAMETERS: usize = 256;
/// Maximum statements in one transaction.
pub const MAX_STATEMENTS: usize = 128;

/// `SQLite` storage classes. Integers retain their full signed 64-bit range.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
	/// SQL NULL.
	Null,
	/// Signed 64-bit integer.
	Integer(i64),
	/// Finite 64-bit floating-point number. NaN and infinities are rejected.
	Real(f64),
	/// UTF-8 text, including embedded NUL characters.
	Text(String),
	/// Arbitrary bytes.
	Blob(Vec<u8>),
}

/// One SQL statement and its ordered `?`/`?NNN` parameter bindings.
#[derive(Debug, Clone, PartialEq)]
pub struct Statement {
	/// Exactly one non-empty SQL statement; multiple statements are rejected.
	pub sql: String,
	/// One value per positional parameter. Strings are never interpolated.
	pub parameters: Vec<Value>,
}

impl Statement {
	/// Builds a parameterized statement. Limits are validated when submitted.
	pub fn new(sql: impl Into<String>, parameters: Vec<Value>) -> Self {
		Self {
			sql: sql.into(),
			parameters,
		}
	}

	/// Encodes a single-statement bridge request.
	pub fn encode(&self) -> Result<Bytes, Error> {
		let mut encoder = Encoder::default();
		encode_statement(&mut encoder, self)?;
		bounded(encoder.finish())
	}

	/// Validates and decodes a single-statement bridge request.
	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		check_size(bytes.len())?;
		let mut decoder = Decoder::new(bytes);
		let statement = decode_statement(&mut decoder)?;
		decoder.finish()?;
		Ok(statement)
	}
}

/// Result from one write statement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExecuteResult {
	/// Number of rows directly changed by this statement.
	pub changes: u64,
	/// `SQLite`'s most recent successful rowid insertion on this connection.
	/// This remains unchanged for statements which do not insert a row.
	pub last_insert_rowid: i64,
}

impl ExecuteResult {
	/// Fixed 16-byte response: changes u64 LE, last insert ID i64 LE.
	pub fn encode(self) -> Bytes {
		let mut bytes = Vec::with_capacity(16);
		bytes.extend_from_slice(&self.changes.to_le_bytes());
		bytes.extend_from_slice(&self.last_insert_rowid.to_le_bytes());
		bytes.into()
	}

	/// Decodes the fixed-width result without losing integer precision.
	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		if bytes.len() != 16 {
			return Err(Error::InvalidArgument);
		}

		Ok(Self {
			changes: u64::from_le_bytes(bytes[..8].try_into().map_err(|_| Error::InvalidArgument)?),
			last_insert_rowid: i64::from_le_bytes(
				bytes[8..].try_into().map_err(|_| Error::InvalidArgument)?,
			),
		})
	}
}

/// Read-only query results. Duplicate column names are preserved positionally.
#[derive(Debug, Clone, PartialEq)]
pub struct QueryResult {
	/// Column names in SELECT order.
	pub columns: Vec<String>,
	/// Each row has exactly `columns.len()` values.
	pub rows: Vec<Vec<Value>>,
}

impl QueryResult {
	/// Encodes names and typed values with explicit counts.
	pub fn encode(&self) -> Result<Bytes, Error> {
		if self.columns.len() > MAX_COLUMNS || self.rows.len() > MAX_ROWS {
			return Err(Error::TooLarge);
		}

		let mut size = 8_usize;

		for column in &self.columns {
			size = size
				.checked_add(column.len())
				.and_then(|size| size.checked_add(4))
				.ok_or(Error::TooLarge)?;
			check_size(size)?;
		}

		let mut encoder = Encoder::default();
		encoder.u32(self.columns.len() as u32);

		for column in &self.columns {
			encoder.bytes(column.as_bytes())?;
		}

		encoder.u32(self.rows.len() as u32);

		for row in &self.rows {
			if row.len() != self.columns.len() {
				return Err(Error::InvalidArgument);
			}

			for value in row {
				size = size
					.checked_add(value_size(value)?)
					.ok_or(Error::TooLarge)?;
				check_size(size)?;
				encode_value(&mut encoder, value)?;
			}
		}

		Ok(encoder.finish())
	}

	/// Rejects malformed tags, non-finite numbers, excess rows and trailing
	/// bytes.
	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		check_size(bytes.len())?;
		let mut decoder = Decoder::new(bytes);
		let count = read_count(&mut decoder, MAX_COLUMNS)?;
		let mut columns = Vec::with_capacity(count);

		for _ in 0..count {
			columns.push(decoder.string()?.to_owned());
		}

		let count = read_count(&mut decoder, MAX_ROWS)?;
		let mut rows = Vec::with_capacity(count);

		for _ in 0..count {
			let mut row = Vec::with_capacity(columns.len());

			for _ in 0..columns.len() {
				row.push(decode_value(&mut decoder)?);
			}

			rows.push(row);
		}

		decoder.finish()?;
		Ok(Self { columns, rows })
	}
}

/// Encodes an atomic batch. All statements commit together or none do.
pub fn encode_transaction(statements: &[Statement]) -> Result<Bytes, Error> {
	let size = transaction_size(statements)?;
	let mut encoder = Encoder::with_capacity(size);
	encoder.u32(statements.len() as u32);

	for statement in statements {
		encode_statement(&mut encoder, statement)?;
	}

	Ok(encoder.finish())
}

fn transaction_size(statements: &[Statement]) -> Result<usize, Error> {
	if statements.is_empty() {
		return Err(Error::InvalidArgument);
	}

	if statements.len() > MAX_STATEMENTS {
		return Err(Error::TooLarge);
	}

	let mut size = 4_usize;

	for statement in statements {
		size = size
			.checked_add(statement_size(statement)?)
			.ok_or(Error::TooLarge)?;
		check_size(size)?;
	}

	Ok(size)
}

/// Decodes the result of an atomic batch, in statement order.
pub fn decode_transaction_result(bytes: &[u8]) -> Result<Vec<ExecuteResult>, Error> {
	check_size(bytes.len())?;
	let mut decoder = Decoder::new(bytes);
	let count = read_count(&mut decoder, MAX_STATEMENTS)?;

	if count == 0 {
		return Err(Error::InvalidArgument);
	}

	let mut results = Vec::with_capacity(count);

	for _ in 0..count {
		results.push(ExecuteResult::decode(decoder.bytes()?)?);
	}

	decoder.finish()?;
	Ok(results)
}

fn statement_size(statement: &Statement) -> Result<usize, Error> {
	if statement.sql.trim().is_empty() || statement.sql.contains('\0') {
		return Err(Error::InvalidArgument);
	}

	if statement.sql.len() > MAX_SQL_BYTES || statement.parameters.len() > MAX_PARAMETERS {
		return Err(Error::TooLarge);
	}

	let mut size = 8 + statement.sql.len();

	for value in &statement.parameters {
		size = size
			.checked_add(value_size(value)?)
			.ok_or(Error::TooLarge)?;
	}

	check_size(size)?;

	Ok(size)
}

fn encode_statement(encoder: &mut Encoder, statement: &Statement) -> Result<(), Error> {
	statement_size(statement)?;
	encoder.bytes(statement.sql.as_bytes())?;
	encoder.u32(statement.parameters.len() as u32);

	for value in &statement.parameters {
		encode_value(encoder, value)?;
	}

	Ok(())
}

fn decode_statement(decoder: &mut Decoder<'_>) -> Result<Statement, Error> {
	let sql = decoder.string()?;

	if sql.len() > MAX_SQL_BYTES {
		return Err(Error::TooLarge);
	}

	let count = read_count(decoder, MAX_PARAMETERS)?;
	let mut parameters = Vec::with_capacity(count);

	for _ in 0..count {
		parameters.push(decode_value(decoder)?);
	}

	let statement = Statement::new(sql, parameters);

	statement_size(&statement)?;
	Ok(statement)
}

fn value_size(value: &Value) -> Result<usize, Error> {
	match value {
		Value::Null => Ok(4),
		Value::Integer(_) => Ok(16),

		Value::Real(value) if value.is_finite() => Ok(16),
		Value::Real(_) => Err(Error::InvalidArgument),
		Value::Text(value) => value.len().checked_add(8).ok_or(Error::TooLarge),
		Value::Blob(value) => value.len().checked_add(8).ok_or(Error::TooLarge),
	}
}

fn encode_value(encoder: &mut Encoder, value: &Value) -> Result<(), Error> {
	value_size(value)?;

	match value {
		Value::Null => encoder.u32(0),
		Value::Integer(value) => {
			encoder.u32(1);
			encoder.bytes(&value.to_le_bytes())?;
		}
		Value::Real(value) => {
			encoder.u32(2);
			encoder.bytes(&value.to_le_bytes())?;
		}
		Value::Text(value) => {
			encoder.u32(3);
			encoder.bytes(value.as_bytes())?;
		}
		Value::Blob(value) => {
			encoder.u32(4);
			encoder.bytes(value)?;
		}
	}

	Ok(())
}

fn decode_value(decoder: &mut Decoder<'_>) -> Result<Value, Error> {
	match decoder.u32()? {
		0 => Ok(Value::Null),
		1 => {
			Ok(Value::Integer(i64::from_le_bytes(
				decoder
					.bytes()?
					.try_into()
					.map_err(|_| Error::InvalidArgument)?,
			)))
		}
		2 => {
			let value = f64::from_le_bytes(
				decoder
					.bytes()?
					.try_into()
					.map_err(|_| Error::InvalidArgument)?,
			);
			if !value.is_finite() {
				return Err(Error::InvalidArgument);
			}

			Ok(Value::Real(value))
		}
		3 => Ok(Value::Text(decoder.string()?.to_owned())),
		4 => Ok(Value::Blob(decoder.bytes()?.to_vec())),
		_ => Err(Error::InvalidArgument),
	}
}

fn read_count(decoder: &mut Decoder<'_>, maximum: usize) -> Result<usize, Error> {
	let count = decoder.u32()? as usize;

	if count > maximum {
		return Err(Error::TooLarge);
	}

	Ok(count)
}
fn check_size(size: usize) -> Result<(), Error> {
	if size > MAX_BYTES {
		Err(Error::TooLarge)
	} else {
		Ok(())
	}
}
fn bounded(bytes: Bytes) -> Result<Bytes, Error> {
	check_size(bytes.len())?;
	Ok(bytes)
}

#[cfg(not(target_arch = "wasm32"))]
mod native;
#[cfg(not(target_arch = "wasm32"))]
pub use native::Database;
#[cfg(not(target_arch = "wasm32"))]
pub use native::DatabaseError;
#[cfg(not(target_arch = "wasm32"))]
pub use native::from_app_dirs;

/// Cached plugin client for interpreter guests. `SQLite` runs in native Rust.
#[cfg(all(target_arch = "wasm32", feature = "guest"))]
pub mod guest {
	use leptoskit_bridge::guest::Client;

	use super::*;

	/// One application database connection managed by the native plugin.
	#[derive(Debug, Clone, Copy)]
	pub struct Sqlite(Client);
	impl Sqlite {
		/// Resolves the installed, permitted `SQLite` plugin.
		pub fn new() -> Result<Self, Error> {
			Client::new("sqlite").map(Self)
		}

		/// Executes exactly one statement with positional bindings.
		pub async fn execute(self, statement: &Statement) -> Result<ExecuteResult, Error> {
			ExecuteResult::decode(&self.0.call(EXECUTE, &statement.encode()?)?.await?)
		}

		/// Queries a read-only statement; use LIMIT/OFFSET for larger results.
		pub async fn query(self, statement: &Statement) -> Result<QueryResult, Error> {
			QueryResult::decode(&self.0.call(QUERY, &statement.encode()?)?.await?)
		}

		/// Executes an atomic batch, rolling back if any statement fails.
		pub async fn transaction(
			self,
			statements: &[Statement],
		) -> Result<Vec<ExecuteResult>, Error> {
			let results = decode_transaction_result(
				&self
					.0
					.call(TRANSACTION, &encode_transaction(statements)?)?
					.await?,
			)?;

			if results.len() != statements.len() {
				return Err(Error::InvalidArgument);
			}

			Ok(results)
		}

		/// Closes the native connection. Later operations reopen it lazily.
		pub async fn close(self) -> Result<(), Error> {
			let result = self.0.call(CLOSE, &[])?.await?;

			if result.is_empty() {
				Ok(())
			} else {
				Err(Error::InvalidArgument)
			}
		}
	}
}
