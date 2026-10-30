//! The connection pool, and transactions within it. Needs a driver: the
//! `postgres` or `sqlite` feature.

use std::sync::Arc;

use leptoskit_db_schema::Dialect;

use crate::Connection;
use crate::DbError;
use crate::Migration;
use crate::MigrationError;
use crate::PoolOptions;
use crate::Read;
use crate::Result;
use crate::Value;
use crate::policy::Access;
use crate::policy::Context;
use crate::sql::Statement;

/// The app's database: a cheap-to-clone handle to a connection pool.
#[derive(Clone)]
pub struct Db {
	pool: Arc<Pool>,
	access: Access,
}

enum Pool {
	#[cfg(feature = "postgres")]
	Postgres(crate::postgres::Pool),
	#[cfg(feature = "sqlite")]
	Sqlite(crate::sqlite::Pool),
}

impl Db {
	/// Connects to `postgres://…`, `sqlite://path`, or `sqlite::memory:`.
	pub async fn connect(url: &str) -> Result<Self> {
		Self::connect_with(url, PoolOptions::default()).await
	}

	/// Connects with explicit pool options.
	// `unused_async_trait_impl` also fires on this inherent method, which is
	// not a trait impl; drop it from the list once clippy stops.
	#[expect(
		clippy::unused_async,
		clippy::unused_async_trait_impl,
		reason = "pools open lazily today; the signature stays async so a driver that connects \
		          eagerly isn't a breaking change"
	)]
	pub async fn connect_with(url: &str, options: PoolOptions) -> Result<Self> {
		let pool = Pool::open(url, options)?;
		Ok(Self {
			pool: Arc::new(pool),
			access: Access::default(),
		})
	}

	/// The same pool, acting for a request: policies read `cx`. Build one
	/// per request once its session is known.
	#[must_use]
	pub fn with_context(&self, cx: Context) -> Self {
		Self {
			pool: Arc::clone(&self.pool),
			access: Access::Context(cx),
		}
	}

	/// The same pool, bypassing every policy: for migrations, jobs, and
	/// other work no request asked for. Say why: the reason goes into the
	/// audit log with every change the handle makes, and so does the
	/// [`Actor`](crate::Actor) of the request this handle acted for, if any.
	#[must_use]
	pub fn system(&self, reason: &'static str) -> Self {
		Self {
			pool: Arc::clone(&self.pool),
			access: Access::System {
				reason,
				actor: self.access.actor(),
			},
		}
	}

	/// The context policies read, unless this handle bypasses them.
	pub fn context(&self) -> Option<&Context> {
		match &self.access {
			Access::Context(cx) => Some(cx),
			Access::System { .. } => None,
		}
	}

	/// Who this handle's changes are recorded as made by.
	pub fn actor(&self) -> Option<crate::Actor> {
		self.access.actor()
	}

	/// The connected database's dialect.
	pub fn dialect(&self) -> Dialect {
		match &*self.pool {
			#[cfg(feature = "postgres")]
			Pool::Postgres(_) => Dialect::Postgres,
			#[cfg(feature = "sqlite")]
			Pool::Sqlite(_) => Dialect::Sqlite,
		}
	}

	/// Runs `work` in a transaction: it commits when `work` returns `Ok`, and
	/// rolls back when it returns `Err` or is dropped part-way, such as when
	/// the request is cancelled.
	///
	/// ```ignore
	/// let post = db.transaction(async |tx| {
	/// 	let post = NewPost { /* … */ }.insert(tx).await?;
	/// 	NewTagging { post_id: post.id, /* … */ }.insert(tx).await?;
	/// 	Ok::<_, DbError>(post)
	/// }).await?;
	/// ```
	pub async fn transaction<T, E>(
		&self,
		work: impl AsyncFnOnce(&Tx) -> std::result::Result<T, E>,
	) -> std::result::Result<T, E>
	where
		E: From<DbError>,
	{
		let tx = self.begin().await?;

		match work(&tx).await {
			Ok(value) => {
				tx.commit().await?;
				Ok(value)
			}
			Err(error) => {
				tx.rollback().await?;
				Err(error)
			}
		}
	}

	/// Starts a transaction to commit or roll back yourself. Prefer
	/// [`Db::transaction`], which can't forget either.
	pub async fn begin(&self) -> Result<Tx> {
		let inner = match &*self.pool {
			#[cfg(feature = "postgres")]
			Pool::Postgres(pool) => TxInner::Postgres(pool.begin().await?),
			#[cfg(feature = "sqlite")]
			Pool::Sqlite(pool) => TxInner::Sqlite(pool.begin().await?),
		};

		let tx = Tx {
			inner,
			access: self.access.clone(),
		};

		#[cfg(feature = "postgres")]
		if matches!(tx.inner, TxInner::Postgres(_))
			&& let Some(actor) = tx.access.actor()
		{
			tx.execute(Statement {
				sql: format!("SET LOCAL \"leptoskit.user_id\" = '{}'", actor.uuid()),
				args: Vec::new(),
			})
			.await?;
		}

		Ok(tx)
	}

	/// Starts a transaction at the strictest isolation level, `SERIALIZABLE`:
	/// it commits only if running it alone, before or after every other
	/// transaction, would have had the same effect. When another transaction
	/// gets in its way, a statement or the commit fails with an error whose
	/// [`DbError::is_contended`] is true, and the whole transaction can run
	/// again. SQLite runs one writer at a time, so its transactions are
	/// serializable already.
	pub async fn begin_serializable(&self) -> Result<Tx> {
		let inner = match &*self.pool {
			#[cfg(feature = "postgres")]
			Pool::Postgres(pool) => {
				TxInner::Postgres(
					pool.begin_with("BEGIN ISOLATION LEVEL SERIALIZABLE")
						.await?,
				)
			}
			#[cfg(feature = "sqlite")]
			Pool::Sqlite(pool) => TxInner::Sqlite(pool.begin().await?),
		};

		let tx = Tx {
			inner,
			access: self.access.clone(),
		};

		#[cfg(feature = "postgres")]
		if matches!(tx.inner, TxInner::Postgres(_)) {
			if let Some(actor) = tx.access.actor() {
				tx.execute(Statement {
					sql: format!("SET LOCAL \"leptoskit.user_id\" = '{}'", actor.uuid()),
					args: Vec::new(),
				})
				.await?;
			}
		}

		Ok(tx)
	}

	/// Applies every migration this database hasn't had yet, in registry
	/// order, each in its own transaction, and returns their names.
	///
	/// Call it at startup with the migrations the server embedded:
	///
	/// ```ignore
	/// static MIGRATIONS: &[Migration] = leptoskit::db::migrations!("../migrations");
	/// db.migrate(MIGRATIONS).await?;
	/// ```
	///
	/// It stops, changing nothing, if an applied migration's SQL has changed or
	/// the database has one this build doesn't know. On Postgres an advisory
	/// lock keeps servers starting together from migrating twice.
	pub async fn migrate(
		&self,
		migrations: &[Migration],
	) -> std::result::Result<Vec<String>, MigrationError> {
		match &*self.pool {
			#[cfg(feature = "postgres")]
			Pool::Postgres(pool) => pool.migrate(migrations).await,
			#[cfg(feature = "sqlite")]
			Pool::Sqlite(pool) => pool.migrate(migrations.to_vec()).await,
		}
	}

	/// The migrations this database has had, in the order they were applied.
	/// Reads only: a database that has never migrated has none.
	pub async fn applied_migrations(&self) -> Result<Vec<String>> {
		let exists = match self.dialect() {
			Dialect::Postgres => {
				"SELECT count(*) FROM pg_catalog.pg_tables WHERE tablename = \
				 '_leptoskit_migrations'"
			}
			Dialect::Sqlite => {
				"SELECT count(*) FROM sqlite_schema WHERE type = 'table' AND name = \
				 '_leptoskit_migrations'"
			}
		};
		let rows = self
			.fetch(Statement {
				sql: exists.to_owned(),
				args: Vec::new(),
			})
			.await?;

		if !matches!(rows.first().and_then(|row| row.first()), Some(Value::Int(count)) if *count > 0)
		{
			return Ok(Vec::new());
		}

		let rows = self
			.fetch(Statement {
				sql: "SELECT \"name\" FROM \"_leptoskit_migrations\" ORDER BY \"applied_at\", \
				      \"name\""
					.to_owned(),
				args: Vec::new(),
			})
			.await?;

		Ok(rows
			.into_iter()
			.filter_map(|row| {
				match row.into_iter().next() {
					Some(Value::Text(name)) => Some(name),
					_ => None,
				}
			})
			.collect())
	}

	/// Runs SQL that may hold several statements, such as a migration, outside
	/// any transaction.
	#[doc(hidden)]
	pub async fn run_script(&self, sql: &str) -> Result<()> {
		match &*self.pool {
			#[cfg(feature = "postgres")]
			Pool::Postgres(pool) => pool.script(sql).await,
			#[cfg(feature = "sqlite")]
			Pool::Sqlite(pool) => pool.script(sql.to_string()).await,
		}
	}
}

impl Pool {
	fn open(url: &str, options: PoolOptions) -> Result<Self> {
		#[cfg(feature = "postgres")]
		if url.starts_with("postgres://") || url.starts_with("postgresql://") {
			return crate::postgres::Pool::connect(url, options).map(Self::Postgres);
		}

		#[cfg(feature = "sqlite")]
		if url.starts_with("sqlite:") {
			return crate::sqlite::Pool::open(url, options).map(Self::Sqlite);
		}

		let scheme = url.split(':').next().unwrap_or_default().to_string();
		Err(DbError::connection(UnsupportedUrl(scheme)))
	}
}

/// A URL for a database this build can't reach.
#[derive(Debug, thiserror::Error)]
#[error(
	"`{0}:` isn't a database this build can connect to; enable the `postgres` or `sqlite` \
	 feature, and use a `postgres://`, `sqlite://`, or `sqlite::memory:` URL"
)]
struct UnsupportedUrl(String);

impl Read for Db {
	fn access(&self) -> &Access {
		&self.access
	}

	fn dialect(&self) -> Dialect {
		Self::dialect(self)
	}

	async fn fetch(&self, statement: Statement) -> Result<Vec<Vec<Value>>> {
		match &*self.pool {
			#[cfg(feature = "postgres")]
			Pool::Postgres(pool) => pool.fetch(statement).await,
			#[cfg(feature = "sqlite")]
			Pool::Sqlite(pool) => pool.fetch(statement).await,
		}
	}
}

impl Connection for Db {
	async fn execute(&self, statement: Statement) -> Result<u64> {
		match &*self.pool {
			#[cfg(feature = "postgres")]
			Pool::Postgres(pool) => pool.execute(statement).await,
			#[cfg(feature = "sqlite")]
			Pool::Sqlite(pool) => pool.execute(statement).await,
		}
	}

	async fn begin_unless_in_transaction(&self) -> Result<Option<Tx>> {
		self.begin().await.map(Some)
	}
}

/// A transaction. Queries through it see each other's writes, and nothing
/// is visible to anyone else until it commits.
pub struct Tx {
	inner: TxInner,
	access: Access,
}

enum TxInner {
	#[cfg(feature = "postgres")]
	Postgres(crate::postgres::Transaction),
	#[cfg(feature = "sqlite")]
	Sqlite(crate::sqlite::Transaction),
}

impl Tx {
	/// The connected database's dialect.
	pub fn dialect(&self) -> Dialect {
		match &self.inner {
			#[cfg(feature = "postgres")]
			TxInner::Postgres(_) => Dialect::Postgres,
			#[cfg(feature = "sqlite")]
			TxInner::Sqlite(_) => Dialect::Sqlite,
		}
	}

	/// Makes every write permanent.
	pub async fn commit(self) -> Result<()> {
		match self.inner {
			#[cfg(feature = "postgres")]
			TxInner::Postgres(tx) => tx.finish("COMMIT").await,
			#[cfg(feature = "sqlite")]
			TxInner::Sqlite(tx) => {
				tx.finish("COMMIT", self.access.actor().map(|actor| actor.uuid()))
					.await
			}
		}
	}

	/// Discards every write.
	pub async fn rollback(self) -> Result<()> {
		match self.inner {
			#[cfg(feature = "postgres")]
			TxInner::Postgres(tx) => tx.finish("ROLLBACK").await,
			#[cfg(feature = "sqlite")]
			TxInner::Sqlite(tx) => tx.finish("ROLLBACK", None).await,
		}
	}
}

impl Read for Tx {
	fn access(&self) -> &Access {
		&self.access
	}

	fn dialect(&self) -> Dialect {
		Self::dialect(self)
	}

	async fn fetch(&self, statement: Statement) -> Result<Vec<Vec<Value>>> {
		match &self.inner {
			#[cfg(feature = "postgres")]
			TxInner::Postgres(tx) => tx.fetch(statement).await,
			#[cfg(feature = "sqlite")]
			TxInner::Sqlite(tx) => tx.fetch(statement).await,
		}
	}
}

impl Connection for Tx {
	async fn execute(&self, statement: Statement) -> Result<u64> {
		match &self.inner {
			#[cfg(feature = "postgres")]
			TxInner::Postgres(tx) => tx.execute(statement).await,
			#[cfg(feature = "sqlite")]
			TxInner::Sqlite(tx) => tx.execute(statement).await,
		}
	}

	fn begin_unless_in_transaction(&self) -> impl Future<Output = Result<Option<Tx>>> + Send {
		std::future::ready(Ok(None))
	}
}
