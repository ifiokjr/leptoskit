use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::time::Duration;

use cap_std::ambient_authority;
use cap_std::fs::Dir;
use cap_std::fs::DirBuilder;
use leptoskit_bridge::Plugin;
use rusqlite::Connection;
use rusqlite::OpenFlags;
use rusqlite::ToSql;
use rusqlite::config::DbConfig;
use rusqlite::hooks::AuthAction;
use rusqlite::hooks::AuthContext;
use rusqlite::hooks::Authorization;
use rusqlite::limits::Limit;
use rusqlite::params_from_iter;
use rusqlite::types::ToSqlOutput;
use rusqlite::types::ValueRef;

use super::*;

/// Native database failures retain the underlying `SQLite` or filesystem cause.
#[derive(Debug, thiserror::Error)]
pub enum DatabaseError {
	/// SQL preparation, binding, execution, constraint, or I/O failure.
	#[error("sqlite operation failed: {0}")]
	Sql(#[from] rusqlite::Error),
	/// Application directory could not be accessed.
	#[error("database directory failed: {0}")]
	Io(#[from] std::io::Error),
	/// Invalid payload, exceeded bound, or prohibited operation.
	#[error("{0}")]
	Contract(#[from] Error),
}

impl DatabaseError {
	fn bridge_error(&self) -> Error {
		match self {
			Self::Contract(error) => *error,

			Self::Io(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
				Error::PermissionDenied
			}

			Self::Sql(rusqlite::Error::SqliteFailure(error, _)) => {
				match error.code {
					rusqlite::ErrorCode::AuthorizationForStatementDenied
					| rusqlite::ErrorCode::PermissionDenied => Error::PermissionDenied,
					rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked => {
						Error::Busy
					}
					rusqlite::ErrorCode::TooBig
					| rusqlite::ErrorCode::OperationInterrupted
					| rusqlite::ErrorCode::DiskFull => Error::TooLarge,
					_ => Error::Native,
				}
			}
			_ => Error::Native,
		}
	}
}

/// Persistent application database. Opening is lazy so generated registration
/// does no database I/O on the UI thread. Calls must be serialized on a worker.
///
/// The host supplies an existing, trusted, application-owned directory.
/// `SQLite` occupies its private `sqlite` child, separate from guest filesystem
/// storage. Native code able to replace this directory is outside this security
/// boundary.
#[derive(Debug)]
pub struct Database {
	data_dir: PathBuf,
	max_bytes: u64,
	connection: Option<Connection>,
	allow_transaction: Arc<AtomicBool>,
	remaining_steps: Arc<AtomicUsize>,
}

/// Factory used by generated native hosts. The cache directory is unused.
pub fn from_app_dirs(data_dir: &Path, _cache_dir: &Path) -> Result<Database, Error> {
	Database::new(data_dir)
}

impl Database {
	/// Retains an absolute application data path without accessing the
	/// filesystem. The host must own the directory; guest input must never
	/// select it.
	pub fn new(data_dir: impl AsRef<Path>) -> Result<Self, Error> {
		Self::with_quota(data_dir, 64 * 1024 * 1024)
	}

	/// Limits the database file to this many bytes (rounded down to whole
	/// pages). `SQLite`'s rollback journal can temporarily use approximately
	/// the same space again. The host must leave that maintenance headroom
	/// available. Existing databases larger than the budget refuse to open.
	pub fn with_quota(data_dir: impl AsRef<Path>, max_bytes: u64) -> Result<Self, Error> {
		let data_dir = data_dir.as_ref();

		if !data_dir.is_absolute() || max_bytes < 512 || max_bytes > u64::from(u32::MAX - 1) * 512 {
			return Err(Error::InvalidArgument);
		}

		Ok(Self {
			data_dir: data_dir.to_owned(),
			max_bytes,
			connection: None,
			allow_transaction: Arc::default(),
			remaining_steps: Arc::default(),
		})
	}

	/// Executes exactly one non-query statement atomically. Multiple
	/// statements, transaction-control SQL, ATTACH, PRAGMA and extensions
	/// are prohibited.
	pub fn execute(&mut self, statement: &Statement) -> Result<ExecuteResult, DatabaseError> {
		let mut results = self.transaction(std::slice::from_ref(statement))?;
		Ok(results.remove(0))
	}

	/// Reads bounded rows. Statements capable of changing the database are
	/// rejected before `SQLite` executes them, including INSERT/UPDATE with
	/// RETURNING.
	pub fn query(&mut self, statement: &Statement) -> Result<QueryResult, DatabaseError> {
		statement_size(statement)?;
		self.open()?;
		self.reset_budget();
		let connection = self.connection.as_ref().ok_or(Error::Native)?;
		let mut prepared = connection.prepare_cached(&statement.sql)?;

		if !prepared.readonly() || prepared.column_count() == 0 {
			return Err(Error::InvalidArgument.into());
		}

		let columns: Vec<String> = prepared
			.column_names()
			.into_iter()
			.map(str::to_owned)
			.collect();

		let mut size = 8 + columns.iter().map(|name| 4 + name.len()).sum::<usize>();
		check_size(size)?;
		let mut cursor = prepared.query(params_from_iter(&statement.parameters))?;
		let mut rows = Vec::new();

		while let Some(row) = cursor.next()? {
			if rows.len() == MAX_ROWS {
				return Err(Error::TooLarge.into());
			}

			let mut values = Vec::with_capacity(columns.len());

			for index in 0..columns.len() {
				let value = row.get_ref(index)?;
				// Check the borrowed `SQLite` value before allocating a Rust
				// copy.
				let bytes = match value {
					ValueRef::Null => 4,
					ValueRef::Integer(_) | ValueRef::Real(_) => 16,
					ValueRef::Text(value) | ValueRef::Blob(value) => 8 + value.len(),
				};

				size = size.checked_add(bytes).ok_or(Error::TooLarge)?;
				check_size(size)?;
				values.push(match value {
					ValueRef::Null => Value::Null,
					ValueRef::Integer(value) => Value::Integer(value),
					ValueRef::Real(value) if value.is_finite() => Value::Real(value),
					ValueRef::Real(_) => return Err(Error::InvalidArgument.into()),
					ValueRef::Text(value) => {
						Value::Text(
							std::str::from_utf8(value)
								.map_err(|_| Error::InvalidArgument)?
								.to_owned(),
						)
					}
					ValueRef::Blob(value) => Value::Blob(value.to_vec()),
				});
			}

			rows.push(values);
		}

		Ok(QueryResult { columns, rows })
	}

	/// Executes a batch in BEGIN IMMEDIATE / COMMIT. Any statement failure or
	/// exceeded VM budget rolls back the entire batch. No transaction remains
	/// open between bridge calls, so unrelated calls cannot accidentally join
	/// it.
	pub fn transaction(
		&mut self,
		statements: &[Statement],
	) -> Result<Vec<ExecuteResult>, DatabaseError> {
		// Validate the complete batch before beginning any side effects.
		transaction_size(statements)?;
		self.open()?;
		self.reset_budget();
		let connection = self.connection.as_ref().ok_or(Error::Native)?;
		self.control("BEGIN IMMEDIATE")?;
		let result = statements
			.iter()
			.map(|statement| {
				let mut prepared = connection.prepare_cached(&statement.sql)?;
				let changes = prepared.execute(params_from_iter(&statement.parameters))?;
				Ok(ExecuteResult {
					changes: changes as u64,
					last_insert_rowid: connection.last_insert_rowid(),
				})
			})
			.collect::<Result<Vec<_>, DatabaseError>>();
		let result = result.and_then(|results| self.control("COMMIT").map(|()| results));

		if result.is_err() {
			// Report the statement's own failure. A failed rollback already
			// discards the connection, so no transaction outlives this call.
			let _ = self.rollback();
		}

		result
	}

	/// Closes the connection, flushing `SQLite`'s normal journal state. The
	/// database stays on disk. A later call opens it again with the same
	/// security settings.
	pub fn close(&mut self) -> Result<(), DatabaseError> {
		if let Some(connection) = self.connection.take()
			&& let Err((connection, error)) = connection.close()
		{
			self.connection = Some(connection);

			return Err(error.into());
		}
		Ok(())
	}

	fn reset_budget(&self) {
		self.remaining_steps.store(1000, Ordering::Relaxed);
	}

	fn control(&self, sql: &str) -> Result<(), DatabaseError> {
		self.allow_transaction.store(true, Ordering::Relaxed);
		let result = self
			.connection
			.as_ref()
			.ok_or(Error::Native)?
			.execute_batch(sql);
		self.allow_transaction.store(false, Ordering::Relaxed);
		Ok(result?)
	}

	fn rollback(&mut self) -> Result<(), DatabaseError> {
		// `SQLite` may already have rolled back when the progress hook
		// interrupted it.
		self.reset_budget();

		if self
			.connection
			.as_ref()
			.is_some_and(|connection| !connection.is_autocommit())
			&& let Err(error) = self.control("ROLLBACK")
		{
			// Never reuse a connection whose transaction outcome is
			// uncertain.
			self.connection.take();

			return Err(error);
		}
		Ok(())
	}

	fn open(&mut self) -> Result<(), DatabaseError> {
		if self.connection.is_some() {
			return Ok(());
		}

		let data_dir = self.private_directory()?;
		let connection = connect(&data_dir)?;

		self.fits_page_budget(&connection)?;
		apply_limits(&connection)?;
		self.guard_sql(&connection)?;
		self.guard_steps(&connection)?;
		self.connection = Some(connection);

		Ok(())
	}

	/// The app-private directory the database lives in, created and checked
	/// before `SQLite` may open anything inside it.
	fn private_directory(&self) -> Result<PathBuf, DatabaseError> {
		let data_dir = std::fs::canonicalize(&self.data_dir)?;
		let root = Dir::open_ambient_dir(&data_dir, ambient_authority())?;
		create_directory(&root)?;

		let metadata = root.symlink_metadata("sqlite")?;

		if !metadata.is_dir() || metadata.file_type().is_symlink() {
			return Err(Error::PermissionDenied.into());
		}

		// Unix permission bits; Windows protects the per-user app data
		// directory with inherited ACLs, which this check does not inspect.
		#[cfg(unix)]
		{
			use cap_std::fs::PermissionsExt as _;

			if metadata.permissions().mode() & 0o077 != 0 {
				return Err(Error::PermissionDenied.into());
			}
		}

		let directory = root.open_dir("sqlite")?;
		check_database_files(&directory, &data_dir)?;

		Ok(data_dir)
	}

	/// Refuses a database that is already larger than the plugin's budget, and
	/// holds `SQLite` to it from here on.
	fn fits_page_budget(&self, connection: &Connection) -> Result<(), DatabaseError> {
		let page_size: u32 = connection.query_row("PRAGMA page_size", [], |row| row.get(0))?;
		let pages = self.max_bytes / u64::from(page_size);
		let count: u32 = connection.query_row("PRAGMA page_count", [], |row| row.get(0))?;

		if pages == 0 || u64::from(count) > pages {
			return Err(Error::TooLarge.into());
		}

		// SQLite reserves pages before writing and rolls back SQLITE_FULL.
		// The authorizer below prevents guests from raising this connection
		// limit.
		let actual: u32 =
			connection.query_row(&format!("PRAGMA max_page_count={pages}"), [], |row| {
				row.get(0)
			})?;

		if u64::from(actual) != pages {
			return Err(Error::TooLarge.into());
		}

		Ok(())
	}

	/// Installs the authorizer that keeps guests out of the filesystem and out
	/// of transactions the host did not open.
	fn guard_sql(&self, connection: &Connection) -> Result<(), DatabaseError> {
		let allowed = Arc::clone(&self.allow_transaction);
		connection.authorizer(Some(move |context: AuthContext<'_>| {
			match context.action {
				AuthAction::Attach { .. }
				| AuthAction::Detach { .. }
				| AuthAction::Pragma { .. }
				| AuthAction::CreateVtable { .. }
				| AuthAction::DropVtable { .. }
				| AuthAction::Unknown { .. } => Authorization::Deny,
				AuthAction::Transaction { .. } | AuthAction::Savepoint { .. }
					if !allowed.load(Ordering::Relaxed) =>
				{
					Authorization::Deny
				}
				AuthAction::Function { function_name }
					if ["load_extension", "readfile", "writefile", "fts3_tokenizer"]
						.iter()
						.any(|name| function_name.eq_ignore_ascii_case(name)) =>
				{
					Authorization::Deny
				}
				_ => Authorization::Allow,
			}
		}))?;

		Ok(())
	}

	/// Stops a call that runs for too many steps.
	fn guard_steps(&self, connection: &Connection) -> Result<(), DatabaseError> {
		let steps = Arc::clone(&self.remaining_steps);
		connection.progress_handler(
			1000,
			Some(move || {
				steps
					.try_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
						value.checked_sub(1)
					})
					.is_err()
			}),
		)?;

		Ok(())
	}
}

impl ToSql for Value {
	fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
		Ok(ToSqlOutput::Borrowed(match self {
			Self::Null => ValueRef::Null,
			Self::Integer(value) => ValueRef::Integer(*value),
			Self::Real(value) => ValueRef::Real(*value),
			Self::Text(value) => ValueRef::Text(value.as_bytes()),
			Self::Blob(value) => ValueRef::Blob(value),
		}))
	}
}

/// Opens the connection with the options that do not depend on its page
/// budget.
fn connect(data_dir: &Path) -> Result<Connection, DatabaseError> {
	let connection = Connection::open_with_flags(
		data_dir.join("sqlite/app.sqlite3"),
		OpenFlags::SQLITE_OPEN_READ_WRITE
			| OpenFlags::SQLITE_OPEN_CREATE
			| OpenFlags::SQLITE_OPEN_NO_MUTEX
			| OpenFlags::SQLITE_OPEN_NOFOLLOW,
	)?;
	connection.busy_timeout(Duration::from_millis(250))?;
	connection.set_prepared_statement_cache_capacity(32);
	connection.set_db_config(DbConfig::SQLITE_DBCONFIG_DEFENSIVE, true)?;
	connection.set_db_config(DbConfig::SQLITE_DBCONFIG_TRUSTED_SCHEMA, false)?;
	connection.set_db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_FKEY, true)?;
	// Keep temporary tables and sorting data out of ambient temporary
	// files.
	connection.execute_batch("PRAGMA temp_store=MEMORY; PRAGMA journal_mode=DELETE;")?;

	Ok(connection)
}

/// Creates the `sqlite` directory, tolerating one that is already there.
fn create_directory(root: &Dir) -> Result<(), DatabaseError> {
	let builder = DirBuilder::new();
	#[cfg(unix)]
	let builder = {
		use cap_std::fs::DirBuilderExt as _;
		let mut builder = builder;
		builder.mode(0o700);
		builder
	};

	match root.create_dir_with("sqlite", &builder) {
		Ok(()) => Ok(()),

		Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
		Err(error) => Err(error.into()),
	}
}

/// Refuses sidecar symlinks and extra hard links in the database directory.
///
/// A fixed database name prevents URI options, traversal and cross-plugin
/// paths. Privileged native code must not mutate this app-private directory
/// concurrently with the plugin.
fn check_database_files(directory: &Dir, data_dir: &Path) -> Result<(), DatabaseError> {
	// Only the Windows branch needs the path, to count links through it.
	#[cfg(not(windows))]
	let _ = data_dir;

	for name in [
		"app.sqlite3",
		"app.sqlite3-journal",
		"app.sqlite3-wal",
		"app.sqlite3-shm",
	] {
		match directory.symlink_metadata(name) {
			Ok(metadata) if !metadata.is_file() || metadata.file_type().is_symlink() => {
				return Err(Error::PermissionDenied.into());
			}

			Ok(metadata) => {
				// A second hard link lets another path read the database or
				// redirects writes into a file the app does not own.
				#[cfg(unix)]
				let links = {
					use cap_std::fs::MetadataExt as _;
					metadata.nlink()
				};
				#[cfg(windows)]
				let links = {
					let _ = metadata;
					u64::from(windows_link_count(&data_dir.join("sqlite").join(name))?)
				};
				#[cfg(not(any(unix, windows)))]
				let links = {
					let _ = metadata;
					1
				};

				if links != 1 {
					return Err(Error::PermissionDenied.into());
				}
			}

			Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
			Err(error) => return Err(error.into()),
		}
	}

	Ok(())
}

/// Holds `SQLite`'s own limits to what the wire format allows.
fn apply_limits(connection: &Connection) -> Result<(), DatabaseError> {
	for (limit, value) in [
		(Limit::SQLITE_LIMIT_LENGTH, MAX_BYTES),
		(Limit::SQLITE_LIMIT_SQL_LENGTH, MAX_SQL_BYTES),
		(Limit::SQLITE_LIMIT_COLUMN, MAX_COLUMNS),
		(Limit::SQLITE_LIMIT_VARIABLE_NUMBER, MAX_PARAMETERS),
		(Limit::SQLITE_LIMIT_ATTACHED, 0),
		(Limit::SQLITE_LIMIT_EXPR_DEPTH, 64),
		(Limit::SQLITE_LIMIT_TRIGGER_DEPTH, 16),
		(Limit::SQLITE_LIMIT_VDBE_OP, 100_000),
	] {
		connection.set_limit(limit, value as i32)?;
	}

	Ok(())
}

impl Plugin for Database {
	fn name(&self) -> &'static str {
		"sqlite"
	}

	fn methods(&self) -> &'static [MethodId] {
		&[EXECUTE, QUERY, TRANSACTION, CLOSE]
	}

	fn invoke(&mut self, method: MethodId, payload: Bytes) -> Result<Bytes, Error> {
		check_size(payload.len())?;

		match method {
			EXECUTE => {
				self.execute(&Statement::decode(&payload)?)
					.map(ExecuteResult::encode)
					.map_err(|error| error.bridge_error())
			}
			QUERY => {
				self.query(&Statement::decode(&payload)?)
					.map_err(|error| error.bridge_error())?
					.encode()
			}
			TRANSACTION => {
				let mut decoder = Decoder::new(&payload);
				let count = read_count(&mut decoder, MAX_STATEMENTS)?;
				let mut statements = Vec::with_capacity(count);
				for _ in 0..count {
					statements.push(decode_statement(&mut decoder)?);
				}

				decoder.finish()?;
				let results = self
					.transaction(&statements)
					.map_err(|error| error.bridge_error())?;
				let mut encoder = Encoder::default();
				encoder.u32(results.len() as u32);
				for result in results {
					encoder.bytes(&result.encode())?;
				}

				Ok(encoder.finish())
			}
			CLOSE => {
				if !payload.is_empty() {
					return Err(Error::InvalidArgument);
				}

				self.close().map_err(|error| error.bridge_error())?;
				Ok(Bytes::new())
			}
			_ => Err(Error::Unavailable),
		}
	}
}

/// Hard-link count of an existing regular file, without following a
/// reparse point. `MetadataExt::number_of_links` is not yet stable.
#[cfg(windows)]
#[allow(unsafe_code)]
fn windows_link_count(path: &Path) -> std::io::Result<u32> {
	use std::os::windows::fs::OpenOptionsExt as _;
	use std::os::windows::io::AsRawHandle as _;

	use windows_sys::Win32::Storage::FileSystem::BY_HANDLE_FILE_INFORMATION;
	use windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT;
	use windows_sys::Win32::Storage::FileSystem::FILE_READ_ATTRIBUTES;
	use windows_sys::Win32::Storage::FileSystem::FILE_SHARE_DELETE;
	use windows_sys::Win32::Storage::FileSystem::FILE_SHARE_READ;
	use windows_sys::Win32::Storage::FileSystem::FILE_SHARE_WRITE;
	use windows_sys::Win32::Storage::FileSystem::GetFileInformationByHandle;
	let file = std::fs::OpenOptions::new()
		.access_mode(FILE_READ_ATTRIBUTES)
		.share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
		.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
		.open(path)?;
	let mut information = std::mem::MaybeUninit::<BY_HANDLE_FILE_INFORMATION>::zeroed();
	// SAFETY: the handle is open for the duration of the call and
	// `information` is a writable, correctly sized output buffer.
	if unsafe { GetFileInformationByHandle(file.as_raw_handle(), information.as_mut_ptr()) } == 0 {
		return Err(std::io::Error::last_os_error());
	}

	// SAFETY: the successful call initialized every field.
	Ok(unsafe { information.assume_init() }.nNumberOfLinks)
}
