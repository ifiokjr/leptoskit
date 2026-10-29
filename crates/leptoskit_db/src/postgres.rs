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
use crate::migrate::pending;
use crate::sql::Statement;

pub(crate) struct Pool(deadpool_postgres::Pool);

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

		Ok(Self(pool))
	}

	async fn client(&self) -> Result<Object> {
		self.0.get().await.map_err(DbError::connection)
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

	pub(crate) async fn begin(&self) -> Result<Transaction> {
		let client = self.client().await?;
		client.batch_execute("BEGIN").await.map_err(error)?;
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
		ref other => {
			return Err(DbError::Decode(DecodeError::custom(format!(
				"column `{}` has Postgres type `{other}`, which leptoskit_db doesn't read yet",
				row.columns()[index].name()
			))));
		}
	};

	Ok(value.unwrap_or(Value::Null))
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
		let client = self.client().await?;
		client
			.execute("SELECT pg_advisory_lock($1)", &[&MIGRATION_LOCK])
			.await
			.map_err(error)?;
		let applied = apply_migrations(&client, migrations).await;
		// The lock also ends with the session, so a failed unlock changes
		// nothing.
		let _ = client
			.execute("SELECT pg_advisory_unlock($1)", &[&MIGRATION_LOCK])
			.await;
		applied
	}
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
	let mut names = Vec::new();

	for migration in pending(&applied, migrations)? {
		client.batch_execute("BEGIN").await.map_err(error)?;
		let result = async {
			client.batch_execute(&migration.sql).await.map_err(error)?;
			client
				.execute(
					"INSERT INTO \"_leptoskit_migrations\" (\"name\", \"checksum\", \
					 \"applied_at\") VALUES ($1, $2, $3)",
					&[
						&migration.name.as_ref(),
						&migration.checksum.as_ref(),
						&crate::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
					],
				)
				.await
				.map_err(error)?;
			client.batch_execute("COMMIT").await.map_err(error)
		}
		.await;

		if let Err(source) = result {
			let _ = client.batch_execute("ROLLBACK").await;
			return Err(MigrationError::Failed {
				name: migration.name.to_string(),
				source,
			});
		}

		names.push(migration.name.to_string());
	}

	Ok(names)
}
