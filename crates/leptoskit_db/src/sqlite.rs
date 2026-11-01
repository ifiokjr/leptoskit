//! SQLite, through `rusqlite`: the same bundled SQLite the on-device plugin
//! uses. Each call runs on tokio's blocking pool.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;

use rusqlite::ErrorCode;
use rusqlite::types::ValueRef;
use tokio::sync::OwnedSemaphorePermit;
use tokio::sync::Semaphore;

use crate::DbError;
use crate::DecodeError;
use crate::Migration;
use crate::MigrationError;
use crate::PoolOptions;
use crate::Result;
use crate::Value;
use crate::migrate::CREATE_MIGRATIONS_TABLE;
use crate::migrate::CREATE_PUSH_TABLE;
use crate::migrate::Record;
use crate::migrate::pending;
use crate::sql::Statement;

/// The text form timestamps take: RFC 3339 in UTC with three fractional
/// digits, matching the schema's `now()` default, so text order is time
/// order.
pub(crate) const TIMESTAMP_FORMAT: &str = "%Y-%m-%dT%H:%M:%S%.3fZ";

#[derive(Clone)]
pub(crate) struct Pool {
	shared: Arc<Shared>,
}

struct Shared {
	target: Target,
	idle: Mutex<Vec<rusqlite::Connection>>,
	permits: Arc<Semaphore>,
	/// Wakes live readers when a commit gets a version.
	live: Arc<crate::live::Hub>,
}

enum Target {
	/// One connection, kept for the pool's life: each in-memory connection
	/// is its own database.
	Memory,
	File(PathBuf),
}

impl Pool {
	pub(crate) fn open(
		url: &str,
		options: PoolOptions,
		live: Arc<crate::live::Hub>,
	) -> Result<Self> {
		let location = url
			.strip_prefix("sqlite://")
			.or_else(|| url.strip_prefix("sqlite:"))
			.unwrap_or(url);
		let (target, size) = if location == ":memory:" {
			(Target::Memory, 1)
		} else {
			(
				Target::File(PathBuf::from(location)),
				options.max_connections.max(1),
			)
		};

		let shared = Shared {
			idle: Mutex::new(Vec::new()),
			permits: Arc::new(Semaphore::new(size)),
			target,
			live,
		};

		// Open one connection now, so a bad path fails at startup.
		let first = shared.connect()?;
		shared.check_in(first);
		Ok(Self {
			shared: Arc::new(shared),
		})
	}

	pub(crate) async fn fetch(&self, statement: Statement) -> Result<Vec<Vec<Value>>> {
		self.run(move |connection| fetch(connection, &statement))
			.await
	}

	pub(crate) async fn execute(&self, statement: Statement) -> Result<u64> {
		self.run(move |connection| execute(connection, &statement))
			.await
	}

	pub(crate) async fn script(&self, sql: String) -> Result<()> {
		self.run(move |connection| connection.execute_batch(&sql).map_err(error))
			.await
	}

	pub(crate) async fn begin(&self) -> Result<Transaction> {
		let permit = self
			.shared
			.permits
			.clone()
			.acquire_owned()
			.await
			.map_err(DbError::connection)?;
		let shared = self.shared.clone();
		let connection = blocking(move || {
			let connection = shared.check_out()?;

			match connection.execute_batch("BEGIN IMMEDIATE") {
				Ok(()) => Ok(connection),
				Err(failure) => {
					shared.check_in(connection);
					Err(error(failure))
				}
			}
		})
		.await?;

		Ok(Transaction {
			shared: self.shared.clone(),
			connection: Arc::new(Mutex::new(Some(connection))),
			permit: Some(permit),
		})
	}

	/// Runs `work` on a pooled connection, on the blocking pool.
	async fn run<R: Send + 'static>(
		&self,
		work: impl FnOnce(&mut rusqlite::Connection) -> Result<R> + Send + 'static,
	) -> Result<R> {
		let permit = self
			.shared
			.permits
			.clone()
			.acquire_owned()
			.await
			.map_err(DbError::connection)?;
		let shared = self.shared.clone();

		blocking(move || {
			let mut connection = shared.check_out()?;
			let before = connection.total_changes();
			let result = work(&mut connection);

			// A write outside a transaction has committed already; its
			// changes to live tables take their version now.
			if connection.total_changes() != before && connection.is_autocommit() {
				shared.seal(&connection);
			}

			shared.check_in(connection);
			drop(permit);
			result
		})
		.await
	}
}

impl Shared {
	fn connect(&self) -> Result<rusqlite::Connection> {
		let connection = match &self.target {
			Target::Memory => rusqlite::Connection::open_in_memory(),
			Target::File(path) => rusqlite::Connection::open(path),
		}
		.map_err(DbError::connection)?;

		let pragmas = match self.target {
			Target::Memory => "PRAGMA foreign_keys = ON;",
			Target::File(_) => {
				"PRAGMA foreign_keys = ON; PRAGMA busy_timeout = 5000; PRAGMA journal_mode = WAL; \
				 PRAGMA synchronous = NORMAL;"
			}
		};

		connection
			.execute_batch(pragmas)
			.map_err(DbError::connection)?;
		#[cfg(feature = "geo")]
		crate::geo::sqlite::register(&connection).map_err(DbError::connection)?;
		#[cfg(feature = "vector")]
		crate::vector::sqlite::register(&connection).map_err(DbError::connection)?;
		Ok(connection)
	}

	/// Gives changes to live tables that have committed without a version,
	/// such as from a write outside a transaction or another process, the
	/// next one. A failure leaves them for the next commit to stamp.
	fn seal(&self, connection: &rusqlite::Connection) {
		if connection.execute_batch("BEGIN IMMEDIATE").is_err() {
			return;
		}

		match crate::live::seal_sqlite(connection, None, None) {
			Ok(version) => {
				if connection.execute_batch("COMMIT").is_ok() && version.is_some() {
					self.live.wake();
				}
			}
			Err(_) => {
				let _ = connection.execute_batch("ROLLBACK");
			}
		}
	}

	fn check_out(&self) -> Result<rusqlite::Connection> {
		let idle = self
			.idle
			.lock()
			.unwrap_or_else(std::sync::PoisonError::into_inner)
			.pop();
		idle.map_or_else(|| self.connect(), Ok)
	}

	/// Returns a connection to the pool, rolling back anything a failed
	/// transaction left open.
	fn check_in(&self, connection: rusqlite::Connection) {
		if !connection.is_autocommit() && connection.execute_batch("ROLLBACK").is_err() {
			return;
		}

		self.idle
			.lock()
			.unwrap_or_else(std::sync::PoisonError::into_inner)
			.push(connection);
	}
}

/// A transaction on one connection, held until it finishes.
pub(crate) struct Transaction {
	shared: Arc<Shared>,
	connection: Arc<Mutex<Option<rusqlite::Connection>>>,
	permit: Option<OwnedSemaphorePermit>,
}

impl Transaction {
	async fn run<R: Send + 'static>(
		&self,
		work: impl FnOnce(&rusqlite::Connection) -> Result<R> + Send + 'static,
	) -> Result<R> {
		let connection = self.connection.clone();

		blocking(move || {
			let guard = connection
				.lock()
				.unwrap_or_else(std::sync::PoisonError::into_inner);
			work(
				guard
					.as_ref()
					.expect("an unfinished transaction has its connection"),
			)
		})
		.await
	}

	pub(crate) async fn fetch(&self, statement: Statement) -> Result<Vec<Vec<Value>>> {
		self.run(move |connection| fetch(connection, &statement))
			.await
	}

	pub(crate) async fn execute(&self, statement: Statement) -> Result<u64> {
		self.run(move |connection| execute(connection, &statement))
			.await
	}

	/// Commits, first giving the transaction's changes to live tables the
	/// next version, recorded as made by `actor` through `origin`.
	pub(crate) async fn commit(
		self,
		actor: Option<uuid::Uuid>,
		origin: Option<String>,
	) -> Result<()> {
		let live = Arc::clone(&self.shared.live);
		let sealed = self
			.finish(move |connection| {
				let version = crate::live::seal_sqlite(connection, actor, origin.as_deref())
					.map_err(error)?;
				connection.execute_batch("COMMIT").map_err(error)?;
				Ok(version)
			})
			.await?;

		if sealed.is_some() {
			live.wake();
		}

		Ok(())
	}

	pub(crate) async fn rollback(self) -> Result<()> {
		self.finish(|connection| connection.execute_batch("ROLLBACK").map_err(error))
			.await
	}

	async fn finish<R: Send + 'static>(
		mut self,
		work: impl FnOnce(&rusqlite::Connection) -> Result<R> + Send + 'static,
	) -> Result<R> {
		let connection = self.connection.clone();
		let shared = self.shared.clone();
		let permit = self.permit.take();

		blocking(move || {
			let connection = connection
				.lock()
				.unwrap_or_else(std::sync::PoisonError::into_inner)
				.take()
				.expect("a transaction finishes once");
			let result = work(&connection);
			shared.check_in(connection);
			drop(permit);
			result
		})
		.await
	}
}

impl Drop for Transaction {
	/// A transaction dropped part-way rolls back before its connection
	/// returns to the pool. The rollback is local and quick, so it runs here.
	fn drop(&mut self) {
		let taken = self
			.connection
			.lock()
			.unwrap_or_else(std::sync::PoisonError::into_inner)
			.take();

		if let Some(connection) = taken {
			self.shared.check_in(connection);
		}
	}
}

async fn blocking<R: Send + 'static>(
	work: impl FnOnce() -> Result<R> + Send + 'static,
) -> Result<R> {
	tokio::task::spawn_blocking(work)
		.await
		.map_err(DbError::query)?
}

fn fetch(connection: &rusqlite::Connection, statement: &Statement) -> Result<Vec<Vec<Value>>> {
	let mut prepared = connection.prepare_cached(&statement.sql).map_err(error)?;
	let columns = prepared.column_count();
	let mut rows = prepared
		.query(rusqlite::params_from_iter(
			statement.args.iter().map(to_sqlite),
		))
		.map_err(error)?;
	let mut decoded = Vec::new();

	while let Some(row) = rows.next().map_err(error)? {
		let values = (0..columns)
			.map(|index| row.get_ref(index).map_err(error).and_then(from_sqlite))
			.collect::<Result<Vec<_>>>()?;
		decoded.push(values);
	}

	Ok(decoded)
}

fn execute(connection: &rusqlite::Connection, statement: &Statement) -> Result<u64> {
	let mut prepared = connection.prepare_cached(&statement.sql).map_err(error)?;
	let changed = prepared
		.execute(rusqlite::params_from_iter(
			statement.args.iter().map(to_sqlite),
		))
		.map_err(error)?;
	Ok(changed as u64)
}

/// A value as SQLite stores it: booleans as integers, UUIDs as 16 bytes,
/// timestamps and dates as text, JSON as text.
fn to_sqlite(value: &Value) -> rusqlite::types::Value {
	use rusqlite::types::Value as Sqlite;

	match value {
		Value::Null => Sqlite::Null,
		Value::Bool(value) => Sqlite::Integer(i64::from(*value)),
		Value::Int(value) => Sqlite::Integer(*value),
		Value::Float(value) => Sqlite::Real(*value),
		Value::Text(value) => Sqlite::Text(value.clone()),
		Value::Bytes(value) => Sqlite::Blob(value.clone()),
		Value::Uuid(value) => Sqlite::Blob(value.as_bytes().to_vec()),
		Value::Timestamp(value) => Sqlite::Text(value.format(TIMESTAMP_FORMAT).to_string()),
		Value::Date(value) => Sqlite::Text(value.format("%Y-%m-%d").to_string()),
		Value::Json(value) => Sqlite::Text(value.to_string()),
	}
}

/// SQLite's storage classes; [`crate::Column`] impls turn them into the
/// declared Rust types.
fn from_sqlite(value: ValueRef<'_>) -> Result<Value> {
	Ok(match value {
		ValueRef::Null => Value::Null,
		ValueRef::Integer(value) => Value::Int(value),
		ValueRef::Real(value) => Value::Float(value),
		ValueRef::Text(text) => {
			Value::Text(
				String::from_utf8(text.to_vec())
					.map_err(|error| DbError::Decode(DecodeError::custom(error)))?,
			)
		}
		ValueRef::Blob(bytes) => Value::Bytes(bytes.to_vec()),
	})
}

/// Maps SQLite's extended constraint codes to their own variants. SQLite
/// reports `UNIQUE constraint failed: table.column` and `CHECK constraint
/// failed: name`, so the table or constraint comes from the message.
fn error(failure: rusqlite::Error) -> DbError {
	let rusqlite::Error::SqliteFailure(code, Some(message)) = &failure else {
		return DbError::query(failure);
	};

	if code.code != ErrorCode::ConstraintViolation {
		return DbError::query(failure);
	}

	let detail = message
		.split_once(": ")
		.map(|(_, detail)| detail.to_string());
	let table = detail
		.as_deref()
		.and_then(|detail| detail.split_once('.'))
		.map(|(table, _)| table.to_string())
		.unwrap_or_default();

	match code.extended_code {
		rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE | rusqlite::ffi::SQLITE_CONSTRAINT_PRIMARYKEY => {
			DbError::Conflict {
				table,
				constraint: None,
				source: Box::new(failure),
			}
		}
		rusqlite::ffi::SQLITE_CONSTRAINT_FOREIGNKEY => {
			DbError::ForeignKey {
				table,
				constraint: None,
				source: Box::new(failure),
			}
		}
		rusqlite::ffi::SQLITE_CONSTRAINT_CHECK => {
			DbError::Check {
				table,
				constraint: detail,
				source: Box::new(failure),
			}
		}
		_ => DbError::query(failure),
	}
}

impl Pool {
	/// Applies the pending migrations on one connection, each in its own
	/// transaction, and returns their names.
	pub(crate) async fn migrate(
		&self,
		migrations: Vec<Migration>,
	) -> std::result::Result<Vec<String>, MigrationError> {
		self.with_connection(move |connection| apply_migrations(connection, &migrations))
			.await
	}

	/// Applies a change without a migration, and records the schema it
	/// leaves.
	pub(crate) async fn push(
		&self,
		migration: Migration,
		schema: String,
	) -> std::result::Result<(), MigrationError> {
		self.with_connection(move |connection| {
			prepare(connection)?;
			apply_with_foreign_keys(connection, &migration, &Record::Pushed(&schema)).map(|_| ())
		})
		.await
	}

	/// Records a migration as applied, without running it, and forgets the
	/// pushes that made its changes.
	pub(crate) async fn adopt(
		&self,
		migration: Migration,
	) -> std::result::Result<(), MigrationError> {
		self.with_connection(move |connection| {
			prepare(connection)?;
			apply_migration(connection, &migration, false, &Record::Adopted).map(|_| ())
		})
		.await
	}

	/// Runs `work` on a connection of its own, off the async threads.
	async fn with_connection<T: Send + 'static>(
		&self,
		work: impl FnOnce(&mut rusqlite::Connection) -> std::result::Result<T, MigrationError>
		+ Send
		+ 'static,
	) -> std::result::Result<T, MigrationError> {
		let permit = self
			.shared
			.permits
			.clone()
			.acquire_owned()
			.await
			.map_err(DbError::connection)?;
		let shared = self.shared.clone();

		tokio::task::spawn_blocking(move || {
			let mut connection = shared.check_out()?;
			let result = work(&mut connection);
			shared.check_in(connection);
			drop(permit);
			result
		})
		.await
		.map_err(|join| MigrationError::Database(DbError::query(join)))?
	}
}

/// Creates the bookkeeping tables a migration needs.
fn prepare(connection: &rusqlite::Connection) -> std::result::Result<(), MigrationError> {
	connection
		.execute_batch(&format!("{CREATE_MIGRATIONS_TABLE}; {CREATE_PUSH_TABLE}"))
		.map_err(error)?;
	Ok(())
}

fn apply_migrations(
	connection: &mut rusqlite::Connection,
	migrations: &[Migration],
) -> std::result::Result<Vec<String>, MigrationError> {
	connection
		.execute_batch(CREATE_MIGRATIONS_TABLE)
		.map_err(error)?;
	let applied = connection
		.prepare("SELECT \"name\", \"checksum\" FROM \"_leptoskit_migrations\"")
		.and_then(|mut statement| {
			statement
				.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
				.collect::<rusqlite::Result<Vec<_>>>()
		})
		.map_err(error)?;
	let pending = pending(&applied, migrations)?;

	if let Some(first) = pending.first()
		&& has_pushes(connection)?
	{
		return Err(MigrationError::Pushed {
			name: first.name.to_string(),
		});
	}

	let mut names = Vec::new();

	for migration in pending {
		if apply_with_foreign_keys(connection, migration, &Record::Applied)? {
			names.push(migration.name.to_string());
		}
	}

	Ok(names)
}

/// Whether the database holds pushed changes no migration records.
fn has_pushes(connection: &rusqlite::Connection) -> std::result::Result<bool, MigrationError> {
	let exists: i64 = connection
		.query_row(
			"SELECT count(*) FROM sqlite_schema WHERE type = 'table' AND name = '_leptoskit_push'",
			[],
			|row| row.get(0),
		)
		.map_err(error)?;

	if exists == 0 {
		return Ok(false);
	}

	let count: i64 = connection
		.query_row("SELECT count(*) FROM \"_leptoskit_push\"", [], |row| {
			row.get(0)
		})
		.map_err(error)?;
	Ok(count > 0)
}

/// Applies one migration with foreign keys off around it when it asks.
fn apply_with_foreign_keys(
	connection: &mut rusqlite::Connection,
	migration: &Migration,
	record: &Record<'_>,
) -> std::result::Result<bool, MigrationError> {
	let foreign_keys_off = migration.foreign_keys_off();

	// The pragma has no effect inside a transaction, so it goes around it.
	if foreign_keys_off {
		connection
			.execute_batch("PRAGMA foreign_keys = OFF")
			.map_err(error)?;
	}

	let result = apply_migration(connection, migration, foreign_keys_off, record);

	if foreign_keys_off {
		connection
			.execute_batch("PRAGMA foreign_keys = ON")
			.map_err(error)?;
	}

	result
}

/// Applies one migration, unless another process got there first. Returns
/// whether it applied it.
fn apply_migration(
	connection: &mut rusqlite::Connection,
	migration: &Migration,
	foreign_keys_off: bool,
	record: &Record<'_>,
) -> std::result::Result<bool, MigrationError> {
	let failed = |source: DbError| {
		MigrationError::Failed {
			name: migration.name.to_string(),
			source,
		}
	};
	let transaction = connection
		.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
		.map_err(error)?;
	let now = crate::now().format(TIMESTAMP_FORMAT).to_string();

	match record {
		Record::Applied => {}
		Record::Pushed(schema) => {
			transaction
				.execute_batch(&migration.sql)
				.map_err(|source| failed(error(source)))?;
			check_foreign_keys(&transaction, migration, foreign_keys_off)?;
			transaction
				.execute(
					"INSERT INTO \"_leptoskit_push\" (\"id\", \"schema\", \"pushed_at\") VALUES \
					 (1, ?1, ?2) ON CONFLICT (\"id\") DO UPDATE SET \"schema\" = \
					 excluded.\"schema\", \"pushed_at\" = excluded.\"pushed_at\"",
					[schema, &now.as_str()],
				)
				.map_err(|source| failed(error(source)))?;
			transaction
				.commit()
				.map_err(|source| failed(error(source)))?;
			return Ok(true);
		}
		Record::Adopted => {
			let forgotten = transaction
				.execute("DELETE FROM \"_leptoskit_push\"", [])
				.map_err(error)?;

			if forgotten == 0 {
				return Err(MigrationError::NothingPushed {
					name: migration.name.to_string(),
				});
			}

			record_applied(&transaction, migration, &now).map_err(failed)?;
			transaction
				.commit()
				.map_err(|source| failed(error(source)))?;
			return Ok(true);
		}
	}

	let already: i64 = transaction
		.query_row(
			"SELECT count(*) FROM \"_leptoskit_migrations\" WHERE \"name\" = ?1",
			[migration.name.as_ref()],
			|row| row.get(0),
		)
		.map_err(error)?;

	if already > 0 {
		return Ok(false);
	}

	transaction
		.execute_batch(&migration.sql)
		.map_err(|source| failed(error(source)))?;
	check_foreign_keys(&transaction, migration, foreign_keys_off)?;
	record_applied(&transaction, migration, &now).map_err(failed)?;
	transaction
		.commit()
		.map_err(|source| failed(error(source)))?;
	Ok(true)
}

/// Fails when a migration that ran with foreign keys off left rows pointing
/// at missing ones.
fn check_foreign_keys(
	transaction: &rusqlite::Transaction<'_>,
	migration: &Migration,
	foreign_keys_off: bool,
) -> std::result::Result<(), MigrationError> {
	if !foreign_keys_off {
		return Ok(());
	}

	let violations: i64 = transaction
		.query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |row| {
			row.get(0)
		})
		.map_err(error)?;

	if violations > 0 {
		return Err(MigrationError::ForeignKeys {
			name: migration.name.to_string(),
			violations,
		});
	}

	Ok(())
}

/// Records `migration` as applied, in its transaction.
fn record_applied(
	transaction: &rusqlite::Transaction<'_>,
	migration: &Migration,
	now: &str,
) -> Result<()> {
	transaction
		.execute(
			"INSERT INTO \"_leptoskit_migrations\" (\"name\", \"checksum\", \"applied_at\") \
			 VALUES (?1, ?2, ?3)",
			[migration.name.as_ref(), migration.checksum.as_ref(), now],
		)
		.map_err(error)?;
	Ok(())
}
