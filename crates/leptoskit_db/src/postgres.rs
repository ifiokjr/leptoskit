//! Postgres, through `tokio-postgres` and a `deadpool` pool.

use std::error::Error;
use std::sync::Arc;

use bytes::BytesMut;
use deadpool_postgres::Manager;
use deadpool_postgres::Object;
use tokio_postgres::Row;
use tokio_postgres::config::SslMode;
use tokio_postgres::error::SqlState;
use tokio_postgres::types::IsNull;
use tokio_postgres::types::ToSql;
use tokio_postgres::types::Type;
use tokio_postgres::types::to_sql_checked;

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

pub(crate) struct Pool {
	pool: deadpool_postgres::Pool,
	/// Where to open the connection that listens for live changes.
	url: String,
}

impl Pool {
	/// Builds the pool; connections open lazily, on first use.
	pub(crate) fn connect(url: &str, options: PoolOptions) -> Result<Self> {
		let config: tokio_postgres::Config = url.parse().map_err(DbError::connection)?;
		let manager = if config.get_ssl_mode() == SslMode::Disable {
			Manager::new(config, tokio_postgres::NoTls)
		} else {
			Manager::new(config, tls()?)
		};
		let pool = deadpool_postgres::Pool::builder(manager)
			.max_size(options.max_connections.max(1))
			.build()
			.map_err(DbError::connection)?;

		Ok(Self {
			pool,
			url: url.to_string(),
		})
	}

	pub(crate) fn url(&self) -> &str {
		&self.url
	}

	async fn client(&self) -> Result<Object> {
		self.pool.get().await.map_err(DbError::connection)
	}

	pub(crate) async fn fetch(&self, statement: Statement) -> Result<Vec<Vec<Value>>> {
		fetch(&self.client().await?, statement).await
	}

	pub(crate) async fn execute(&self, statement: Statement) -> Result<u64> {
		execute(&self.client().await?, statement).await
	}

	pub(crate) async fn script(&self, sql: &str) -> Result<()> {
		self.client().await?.batch_execute(sql).await.map_err(error)
	}

	/// Runs a statement in a transaction that first makes `settings`, so
	/// row-level security policies read them.
	pub(crate) async fn fetch_secured(
		&self,
		settings: &[(String, String)],
		statement: Statement,
	) -> Result<Vec<Vec<Value>>> {
		let tx = self.begin().await?;
		tx.apply_settings(settings).await?;

		match tx.fetch(statement).await {
			Ok(rows) => {
				tx.finish("COMMIT").await?;
				Ok(rows)
			}
			Err(error) => {
				tx.finish("ROLLBACK").await?;
				Err(error)
			}
		}
	}

	/// [`Pool::fetch_secured`], counting the rows the statement changed.
	pub(crate) async fn execute_secured(
		&self,
		settings: &[(String, String)],
		statement: Statement,
	) -> Result<u64> {
		let tx = self.begin().await?;
		tx.apply_settings(settings).await?;

		match tx.execute(statement).await {
			Ok(changed) => {
				tx.finish("COMMIT").await?;
				Ok(changed)
			}
			Err(error) => {
				tx.finish("ROLLBACK").await?;
				Err(error)
			}
		}
	}

	pub(crate) async fn begin(&self) -> Result<Transaction> {
		self.begin_with("BEGIN").await
	}

	/// Starts a transaction with `command`, such as `BEGIN ISOLATION LEVEL
	/// SERIALIZABLE`.
	pub(crate) async fn begin_with(&self, command: &str) -> Result<Transaction> {
		let client = self.client().await?;
		client.batch_execute(command).await.map_err(error)?;
		Ok(Transaction {
			client: Some(client),
		})
	}
}

/// A transaction on one pooled connection.
pub(crate) struct Transaction {
	/// Taken when the transaction finishes, so that `Drop` knows.
	client: Option<Object>,
}

impl Transaction {
	fn client(&self) -> &Object {
		self.client
			.as_ref()
			.expect("an unfinished transaction has its connection")
	}

	pub(crate) async fn fetch(&self, statement: Statement) -> Result<Vec<Vec<Value>>> {
		fetch(self.client(), statement).await
	}

	pub(crate) async fn execute(&self, statement: Statement) -> Result<u64> {
		execute(self.client(), statement).await
	}

	/// Sets each `(name, value)` until the transaction ends, in one
	/// statement: `SELECT set_config($1, $2, true), …`.
	pub(crate) async fn apply_settings(&self, settings: &[(String, String)]) -> Result<()> {
		if settings.is_empty() {
			return Ok(());
		}

		let calls: Vec<String> = (0..settings.len())
			.map(|index| format!("set_config(${}, ${}, true)", index * 2 + 1, index * 2 + 2))
			.collect();
		let args = settings
			.iter()
			.flat_map(|(name, value)| [Value::Text(name.clone()), Value::Text(value.clone())])
			.collect();
		self.fetch(Statement {
			sql: format!("SELECT {}", calls.join(", ")),
			args,
		})
		.await?;
		Ok(())
	}

	pub(crate) async fn finish(mut self, command: &str) -> Result<()> {
		let client = self.client.take().expect("a transaction finishes once");
		client.batch_execute(command).await.map_err(error)
	}
}

impl Drop for Transaction {
	/// A transaction dropped part-way closes its connection instead of
	/// returning it to the pool: closing rolls back, and the next user never
	/// inherits half a transaction.
	fn drop(&mut self) {
		if let Some(client) = self.client.take() {
			drop(Object::take(client));
		}
	}
}

async fn fetch(client: &Object, statement: Statement) -> Result<Vec<Vec<Value>>> {
	let prepared = client.prepare_cached(&statement.sql).await.map_err(error)?;
	let params: Vec<Param> = statement.args.into_iter().map(Param).collect();
	let refs: Vec<&(dyn ToSql + Sync)> = params
		.iter()
		.map(|param| param as &(dyn ToSql + Sync))
		.collect();
	let rows = client.query(&prepared, &refs).await.map_err(error)?;
	rows.iter().map(decode).collect()
}

async fn execute(client: &Object, statement: Statement) -> Result<u64> {
	let prepared = client.prepare_cached(&statement.sql).await.map_err(error)?;
	let params: Vec<Param> = statement.args.into_iter().map(Param).collect();
	let refs: Vec<&(dyn ToSql + Sync)> = params
		.iter()
		.map(|param| param as &(dyn ToSql + Sync))
		.collect();
	client.execute(&prepared, &refs).await.map_err(error)
}

/// TLS with the webpki roots, for hosted databases.
fn tls() -> Result<tokio_postgres_rustls::MakeRustlsConnect> {
	let mut roots = rustls::RootCertStore::empty();
	roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
	let config = rustls::ClientConfig::builder_with_provider(Arc::new(
		rustls::crypto::ring::default_provider(),
	))
	.with_safe_default_protocol_versions()
	.map_err(DbError::connection)?
	.with_root_certificates(roots)
	.with_no_client_auth();

	Ok(tokio_postgres_rustls::MakeRustlsConnect::new(config))
}

/// A value bound to whatever type the prepared statement says its
/// placeholder has, so integers narrow (checked) to `int2` or `int4`, and
/// `NULL` needs no type of its own.
#[derive(Debug)]
struct Param(Value);

impl ToSql for Param {
	to_sql_checked!();

	fn to_sql(
		&self,
		ty: &Type,
		out: &mut BytesMut,
	) -> std::result::Result<IsNull, Box<dyn Error + Sync + Send>> {
		match &self.0 {
			Value::Null => Ok(IsNull::Yes),
			Value::Bool(value) => value.to_sql_checked(ty, out),
			Value::Int(value) => {
				match *ty {
					Type::INT2 => i16::try_from(*value)?.to_sql_checked(ty, out),
					Type::INT4 => i32::try_from(*value)?.to_sql_checked(ty, out),
					Type::FLOAT4 => (*value as f32).to_sql_checked(ty, out),
					Type::FLOAT8 => (*value as f64).to_sql_checked(ty, out),
					_ => value.to_sql_checked(ty, out),
				}
			}
			Value::Float(value) if *ty == Type::FLOAT4 => (*value as f32).to_sql_checked(ty, out),
			Value::Float(value) => value.to_sql_checked(ty, out),
			Value::Text(value) => value.as_str().to_sql_checked(ty, out),
			// A shape, held as GeoPackage's encoding, goes to PostGIS as the
			// EWKB its binary input reads.
			#[cfg(feature = "geo")]
			Value::Bytes(value) if is_geospatial(ty) => {
				out.extend_from_slice(&crate::geo::codec::to_ewkb(value)?);
				Ok(IsNull::No)
			}
			Value::Bytes(value) => value.as_slice().to_sql_checked(ty, out),
			Value::Uuid(value) => value.to_sql_checked(ty, out),
			Value::Timestamp(value) if *ty == Type::TIMESTAMP => {
				value.naive_utc().to_sql_checked(ty, out)
			}
			Value::Timestamp(value) => value.to_sql_checked(ty, out),
			Value::Date(value) => value.to_sql_checked(ty, out),
			Value::Json(value) => value.to_sql_checked(ty, out),
		}
	}

	fn accepts(_: &Type) -> bool {
		true
	}
}

/// A row's values, decoded by each column's reported type.
fn decode(row: &Row) -> Result<Vec<Value>> {
	(0..row.len())
		.map(|index| decode_column(row, index))
		.collect()
}

fn decode_column(row: &Row, index: usize) -> Result<Value> {
	let ty = row.columns()[index].type_();
	let value = match *ty {
		Type::BOOL => get::<bool>(row, index)?.map(Value::Bool),
		Type::INT2 => get::<i16>(row, index)?.map(|value| Value::Int(value.into())),
		Type::INT4 => get::<i32>(row, index)?.map(|value| Value::Int(value.into())),
		Type::INT8 => get::<i64>(row, index)?.map(Value::Int),
		Type::FLOAT4 => get::<f32>(row, index)?.map(|value| Value::Float(value.into())),
		Type::FLOAT8 => get::<f64>(row, index)?.map(Value::Float),
		Type::TEXT | Type::VARCHAR | Type::BPCHAR | Type::NAME => {
			get::<String>(row, index)?.map(Value::Text)
		}
		Type::BYTEA => get::<Vec<u8>>(row, index)?.map(Value::Bytes),
		Type::UUID => get::<uuid::Uuid>(row, index)?.map(Value::Uuid),
		Type::TIMESTAMPTZ => {
			get::<chrono::DateTime<chrono::Utc>>(row, index)?.map(Value::Timestamp)
		}
		Type::TIMESTAMP => {
			get::<chrono::NaiveDateTime>(row, index)?.map(|value| Value::Timestamp(value.and_utc()))
		}
		Type::DATE => get::<chrono::NaiveDate>(row, index)?.map(Value::Date),
		Type::JSON | Type::JSONB => get::<serde_json::Value>(row, index)?.map(Value::Json),
		#[cfg(feature = "geo")]
		ref other if is_geospatial(other) => {
			get::<Raw>(row, index)?
				.map(|raw| crate::geo::codec::ewkb_to_gpkg(&raw.0))
				.transpose()
				.map_err(|error| {
					DbError::Decode(DecodeError::custom(format!(
						"column `{}`: {error}",
						row.columns()[index].name()
					)))
				})?
				.map(Value::Bytes)
		}
		ref other => {
			return Err(DbError::Decode(DecodeError::custom(format!(
				"column `{}` has Postgres type `{other}`, which leptoskit_db doesn't read yet",
				row.columns()[index].name()
			))));
		}
	};

	Ok(value.unwrap_or(Value::Null))
}

/// Whether a type is PostGIS's: its binary form is EWKB.
#[cfg(feature = "geo")]
fn is_geospatial(ty: &Type) -> bool {
	matches!(ty.name(), "geometry" | "geography")
}

/// A value's binary form, whatever its type, for the extensions' types
/// whose formats the database layer reads itself.
#[cfg(feature = "geo")]
struct Raw(Vec<u8>);

#[cfg(feature = "geo")]
impl<'a> tokio_postgres::types::FromSql<'a> for Raw {
	fn from_sql(
		_: &Type,
		raw: &'a [u8],
	) -> std::result::Result<Self, Box<dyn Error + Sync + Send>> {
		Ok(Self(raw.to_vec()))
	}

	fn accepts(ty: &Type) -> bool {
		is_geospatial(ty)
	}
}

fn get<'a, T: tokio_postgres::types::FromSql<'a>>(row: &'a Row, index: usize) -> Result<Option<T>> {
	row.try_get(index).map_err(DbError::query)
}

/// Maps Postgres's error codes for broken constraints to their own variants.
fn error(error: tokio_postgres::Error) -> DbError {
	let Some(database) = error.as_db_error() else {
		return if error.is_closed() {
			DbError::connection(error)
		} else {
			DbError::query(error)
		};
	};
	let table = database.table().unwrap_or_default().to_string();
	let constraint = database.constraint().map(ToString::to_string);
	let code = database.code().clone();

	if code == SqlState::UNIQUE_VIOLATION {
		DbError::Conflict {
			table,
			constraint,
			source: Box::new(error),
		}
	} else if code == SqlState::FOREIGN_KEY_VIOLATION {
		DbError::ForeignKey {
			table,
			constraint,
			source: Box::new(error),
		}
	} else if code == SqlState::INSUFFICIENT_PRIVILEGE
		&& database.message().contains("row-level security")
	{
		// Postgres's own policies refused a write the database layer let
		// through; its message names the table.
		let message = database.message().to_string();
		DbError::Forbidden {
			table: table_from_message(&message).unwrap_or(table),
			message,
		}
	} else if code == SqlState::CHECK_VIOLATION {
		DbError::Check {
			table,
			constraint,
			source: Box::new(error),
		}
	} else {
		DbError::query(error)
	}
}

/// The table a row-level security refusal names: `… for table "posts"`.
fn table_from_message(message: &str) -> Option<String> {
	let (_, rest) = message.rsplit_once("for table \"")?;
	rest.split('"').next().map(ToString::to_string)
}

/// The advisory lock that keeps two servers from migrating at once
/// (`lkdbmigr` in ASCII).
const MIGRATION_LOCK: i64 = 0x6c6b_6462_6d69_6772;

impl Pool {
	/// Applies the pending migrations while holding the advisory lock, each in
	/// its own transaction, and returns their names.
	pub(crate) async fn migrate(
		&self,
		migrations: &[Migration],
	) -> std::result::Result<Vec<String>, MigrationError> {
		self.locked(async |client| apply_migrations(client, migrations).await)
			.await
	}

	/// Applies a change without a migration, and records the schema it
	/// leaves.
	pub(crate) async fn push(
		&self,
		migration: &Migration,
		schema: &str,
	) -> std::result::Result<(), MigrationError> {
		self.locked(async |client| {
			prepare(client).await?;
			apply_one(client, migration, &Record::Pushed(schema)).await
		})
		.await
	}

	/// Records a migration as applied, without running it, and forgets the
	/// pushes that made its changes.
	pub(crate) async fn adopt(
		&self,
		migration: &Migration,
	) -> std::result::Result<(), MigrationError> {
		self.locked(async |client| {
			prepare(client).await?;
			apply_one(client, migration, &Record::Adopted).await
		})
		.await
	}

	/// Runs `work` holding the migration lock, which keeps servers starting
	/// together from migrating twice.
	async fn locked<T>(
		&self,
		work: impl AsyncFnOnce(&Object) -> std::result::Result<T, MigrationError>,
	) -> std::result::Result<T, MigrationError> {
		let client = self.client().await?;
		client
			.execute("SELECT pg_advisory_lock($1)", &[&MIGRATION_LOCK])
			.await
			.map_err(error)?;
		let result = work(&client).await;
		// The lock also ends with the session, so a failed unlock changes
		// nothing.
		let _ = client
			.execute("SELECT pg_advisory_unlock($1)", &[&MIGRATION_LOCK])
			.await;
		result
	}
}

/// Creates the bookkeeping tables a migration needs.
async fn prepare(client: &Object) -> std::result::Result<(), MigrationError> {
	client
		.batch_execute(&format!("{CREATE_MIGRATIONS_TABLE}; {CREATE_PUSH_TABLE}"))
		.await
		.map_err(error)?;
	Ok(())
}

async fn apply_migrations(
	client: &Object,
	migrations: &[Migration],
) -> std::result::Result<Vec<String>, MigrationError> {
	client
		.batch_execute(CREATE_MIGRATIONS_TABLE)
		.await
		.map_err(error)?;
	let rows = client
		.query(
			"SELECT \"name\", \"checksum\" FROM \"_leptoskit_migrations\"",
			&[],
		)
		.await
		.map_err(error)?;
	let applied: Vec<(String, String)> = rows.iter().map(|row| (row.get(0), row.get(1))).collect();
	let pending = pending(&applied, migrations)?;

	if let Some(first) = pending.first()
		&& has_pushes(client).await?
	{
		return Err(MigrationError::Pushed {
			name: first.name.to_string(),
		});
	}

	let mut names = Vec::new();

	for migration in pending {
		apply_one(client, migration, &Record::Applied).await?;
		names.push(migration.name.to_string());
	}

	Ok(names)
}

/// Whether the database holds pushed changes no migration records.
async fn has_pushes(client: &Object) -> std::result::Result<bool, MigrationError> {
	let exists: bool = client
		.query_one("SELECT to_regclass('\"_leptoskit_push\"') IS NOT NULL", &[])
		.await
		.map_err(error)?
		.get(0);

	if !exists {
		return Ok(false);
	}

	let count: i64 = client
		.query_one("SELECT count(*) FROM \"_leptoskit_push\"", &[])
		.await
		.map_err(error)?
		.get(0);
	Ok(count > 0)
}

/// Runs one migration in its own transaction, and records it as `record`
/// says.
async fn apply_one(
	client: &Object,
	migration: &Migration,
	record: &Record<'_>,
) -> std::result::Result<(), MigrationError> {
	let failed = |source| {
		MigrationError::Failed {
			name: migration.name.to_string(),
			source,
		}
	};
	let now = crate::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
	// Migrations change every row they mean to, whatever row-level
	// security policies say.
	client
		.batch_execute(&format!(
			"BEGIN; SELECT set_config('{}', 'on', true)",
			leptoskit_db_schema::BYPASS_SETTING
		))
		.await
		.map_err(error)?;
	let result = async {
		match record {
			Record::Applied => {
				client.batch_execute(&migration.sql).await.map_err(error)?;
			}
			Record::Pushed(schema) => {
				client.batch_execute(&migration.sql).await.map_err(error)?;
				client
					.execute(
						"INSERT INTO \"_leptoskit_push\" (\"id\", \"schema\", \"pushed_at\") \
						 VALUES (1, $1, $2) ON CONFLICT (\"id\") DO UPDATE SET \"schema\" = \
						 EXCLUDED.\"schema\", \"pushed_at\" = EXCLUDED.\"pushed_at\"",
						&[schema, &now],
					)
					.await
					.map_err(error)?;
				client.batch_execute("COMMIT").await.map_err(error)?;
				return Ok(false);
			}
			Record::Adopted => {
				let forgotten = client
					.execute("DELETE FROM \"_leptoskit_push\"", &[])
					.await
					.map_err(error)?;

				if forgotten == 0 {
					return Ok(true);
				}
			}
		}

		client
			.execute(
				"INSERT INTO \"_leptoskit_migrations\" (\"name\", \"checksum\", \"applied_at\") \
				 VALUES ($1, $2, $3)",
				&[&migration.name.as_ref(), &migration.checksum.as_ref(), &now],
			)
			.await
			.map_err(error)?;
		client.batch_execute("COMMIT").await.map_err(error)?;
		Ok(false)
	}
	.await;

	match result {
		Ok(false) => Ok(()),
		Ok(true) => {
			let _ = client.batch_execute("ROLLBACK").await;
			Err(MigrationError::NothingPushed {
				name: migration.name.to_string(),
			})
		}
		Err(source) => {
			let _ = client.batch_execute("ROLLBACK").await;
			Err(failed(source))
		}
	}
}

/// The connection that listens for live changes, from every server sharing
/// the database, and hands them to the hub. It reconnects when the
/// connection drops, telling subscribers changes may have been missed, and
/// stops when the database's last handle is dropped.
pub(crate) struct Listener {
	_stop: tokio::sync::oneshot::Sender<()>,
}

impl Listener {
	/// Connects and listens, and returns once `LISTEN` has taken effect, so
	/// that a subscriber made after it hears every change committed after it.
	pub(crate) async fn start(url: &str, hub: std::sync::Weak<crate::live::Hub>) -> Result<Self> {
		let config: tokio_postgres::Config = url.parse().map_err(DbError::connection)?;
		let (ready, listening) = tokio::sync::oneshot::channel();
		let (stop, stopped) = tokio::sync::oneshot::channel();
		tokio::spawn(listen(config, hub, ready, stopped));

		match listening.await {
			Ok(Ok(())) => Ok(Self { _stop: stop }),
			Ok(Err(error)) => Err(error),
			Err(_) => Err(DbError::connection(ListenerEnded)),
		}
	}
}

/// The listening connection ended.
#[derive(Debug, thiserror::Error)]
#[error("the connection listening for live changes ended")]
struct ListenerEnded;

/// What happened on a listening connection.
enum Heard<'a> {
	/// `LISTEN` took effect.
	Listening,
	/// A notification's payload.
	Payload(&'a str),
}

/// Why a listening connection stopped.
enum Outcome {
	/// The hub is gone, so nobody needs changes any more.
	Stopped,
	/// The connection failed or ended.
	Failed(DbError),
}

/// Listens until the hub is gone, reconnecting with a growing delay.
async fn listen(
	config: tokio_postgres::Config,
	hub: std::sync::Weak<crate::live::Hub>,
	ready: tokio::sync::oneshot::Sender<Result<()>>,
	mut stop: tokio::sync::oneshot::Receiver<()>,
) {
	use crate::live::Event;

	let mut ready = Some(ready);
	let mut delay = std::time::Duration::from_millis(100);

	loop {
		let mut assembler = crate::live::Assembler::default();
		let mut heard = |heard: Heard<'_>| -> bool {
			let Some(hub) = hub.upgrade() else {
				return false;
			};

			match heard {
				Heard::Listening => {
					delay = std::time::Duration::from_millis(100);

					match ready.take() {
						Some(ready) => {
							let _ = ready.send(Ok(()));
						}
						// Whatever committed while reconnecting was missed.
						None => hub.send(Event::Lost),
					}
				}
				Heard::Payload(payload) => {
					match assembler.push(payload) {
						Ok(Some(changes)) => hub.send(Event::Changes(changes.into())),
						Ok(None) => {}
						Err(_) => {
							assembler.clear();
							hub.send(Event::Lost);
						}
					}
				}
			}

			true
		};

		let error = match session(&config, &mut stop, &mut heard).await {
			Outcome::Stopped => return,
			Outcome::Failed(error) => error,
		};

		// The first connection's failure is the caller's to see; a later
		// one is retried.
		if let Some(ready) = ready.take() {
			let _ = ready.send(Err(error));
			return;
		}

		match hub.upgrade() {
			Some(hub) => hub.send(Event::Lost),
			None => return,
		}

		let mut sleep = std::pin::pin!(tokio::time::sleep(delay));
		let stopped = std::future::poll_fn(|cx| {
			if std::pin::Pin::new(&mut stop).poll(cx).is_ready() {
				return std::task::Poll::Ready(true);
			}

			sleep.as_mut().poll(cx).map(|()| false)
		})
		.await;

		if stopped {
			return;
		}

		delay = (delay * 2).min(std::time::Duration::from_secs(30));
	}
}

/// One listening connection, until it fails or `stop` fires.
async fn session(
	config: &tokio_postgres::Config,
	stop: &mut tokio::sync::oneshot::Receiver<()>,
	heard: &mut (dyn FnMut(Heard<'_>) -> bool + Send),
) -> Outcome {
	if config.get_ssl_mode() == SslMode::Disable {
		match config.connect(tokio_postgres::NoTls).await {
			Ok((client, connection)) => drive(client, connection, stop, heard).await,
			Err(failure) => Outcome::Failed(error(failure)),
		}
	} else {
		let tls = match tls() {
			Ok(tls) => tls,
			Err(error) => return Outcome::Failed(error),
		};

		match config.connect(tls).await {
			Ok((client, connection)) => drive(client, connection, stop, heard).await,
			Err(failure) => Outcome::Failed(error(failure)),
		}
	}
}

/// Runs `LISTEN` and hands over notifications, polling the connection
/// itself, which is where notifications arrive.
async fn drive<S, T>(
	client: tokio_postgres::Client,
	mut connection: tokio_postgres::Connection<S, T>,
	stop: &mut tokio::sync::oneshot::Receiver<()>,
	heard: &mut (dyn FnMut(Heard<'_>) -> bool + Send),
) -> Outcome
where
	S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
	T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
	use std::task::Poll;

	let command = format!("LISTEN {}", crate::live::CHANNEL);
	let mut listen = std::pin::pin!(client.batch_execute(&command));
	let mut listening = false;

	std::future::poll_fn(|cx| {
		if std::pin::Pin::new(&mut *stop).poll(cx).is_ready() {
			return Poll::Ready(Outcome::Stopped);
		}

		loop {
			match connection.poll_message(cx) {
				Poll::Ready(Some(Ok(tokio_postgres::AsyncMessage::Notification(notification)))) => {
					if !heard(Heard::Payload(notification.payload())) {
						return Poll::Ready(Outcome::Stopped);
					}
				}
				Poll::Ready(Some(Ok(_))) => {}
				Poll::Ready(Some(Err(failure))) => {
					return Poll::Ready(Outcome::Failed(error(failure)));
				}
				Poll::Ready(None) => {
					return Poll::Ready(Outcome::Failed(DbError::connection(ListenerEnded)));
				}
				Poll::Pending => break,
			}
		}

		if !listening && let Poll::Ready(result) = listen.as_mut().poll(cx) {
			if let Err(failure) = result {
				return Poll::Ready(Outcome::Failed(error(failure)));
			}

			listening = true;

			if !heard(Heard::Listening) {
				return Poll::Ready(Outcome::Stopped);
			}
		}

		Poll::Pending
	})
	.await
}
