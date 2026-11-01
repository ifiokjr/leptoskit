//! The connection pool, and transactions within it. Needs a driver: the
//! `postgres` or `sqlite` feature.

use std::sync::Arc;
#[cfg(feature = "sqlite")]
use std::sync::Mutex;

use leptoskit_db_schema::Dialect;

use crate::Connection;
use crate::DbError;
use crate::Migration;
use crate::MigrationError;
use crate::PoolOptions;
use crate::Read;
use crate::Result;
use crate::Value;
use crate::live::Hub;
use crate::policy::Access;
use crate::policy::Context;
use crate::sql::Statement;

/// The app's database: a cheap-to-clone handle to a connection pool.
#[derive(Clone)]
pub struct Db {
	pool: Arc<Pool>,
	pub(crate) live: Arc<Hub>,
	access: Access,
	/// Whether every statement runs in a transaction that tells Postgres
	/// who is asking, for its row-level security policies.
	row_security: bool,
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
		let live = Arc::new(Hub::new());
		let pool = Pool::open(url, options, &live)?;
		Ok(Self {
			pool: Arc::new(pool),
			live,
			access: Access::default(),
			row_security: false,
		})
	}

	/// The same pool, telling Postgres who each statement acts for, so the
	/// row-level security policies that `[db] policies = "rls"` migrations
	/// create filter every statement, raw SQL included. Call it once, right
	/// after connecting, when the migrations were made that way.
	///
	/// Each transaction then begins by setting every registered
	/// [`Setting`](crate::Setting) from the request's context, empty when it
	/// has none, and whether the handle bypasses policies
	/// ([`Db::system`]). Statements outside a transaction each get one.
	///
	/// # Errors
	///
	/// On SQLite, which has no row-level security: there the database layer
	/// alone enforces policies.
	pub fn with_row_security(&self) -> Result<Self> {
		if self.dialect() != Dialect::Postgres {
			return Err(DbError::connection(NoRowSecurity));
		}

		Ok(Self {
			row_security: true,
			..self.clone()
		})
	}

	/// Whether statements tell Postgres who they act for.
	pub fn has_row_security(&self) -> bool {
		self.row_security
	}

	/// The settings each transaction makes, under row-level security.
	#[cfg(feature = "postgres")]
	fn row_settings(&self) -> Result<Option<Vec<(String, String)>>> {
		if !self.row_security {
			return Ok(None);
		}

		Ok(Some(crate::settings::for_access(&self.access)?))
	}

	/// The same pool, acting for a request: policies read `cx`. Build one
	/// per request once its session is known.
	#[must_use]
	pub fn with_context(&self, cx: Context) -> Self {
		Self {
			pool: Arc::clone(&self.pool),
			live: Arc::clone(&self.live),
			access: Access::Context(cx),
			row_security: self.row_security,
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
			live: Arc::clone(&self.live),
			access: Access::System {
				reason,
				actor: self.access.actor(),
			},
			row_security: self.row_security,
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
		Read::dialect(self)
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
			Pool::Postgres(pool) => {
				let tx = pool.begin().await?;

				if let Some(settings) = self.row_settings()? {
					tx.apply_settings(&settings).await?;
				}

				TxInner::Postgres(tx)
			}
			#[cfg(feature = "sqlite")]
			Pool::Sqlite(pool) => TxInner::Sqlite(pool.begin().await?),
		};

		self.started(inner).await
	}

	/// A transaction that has begun, telling Postgres who acts in it, so
	/// that the change log records the commit's actor.
	async fn started(&self, inner: TxInner) -> Result<Tx> {
		let tx = Tx {
			inner,
			access: self.access.clone(),
			row_security: self.row_security,
			#[cfg(feature = "sqlite")]
			origin: Mutex::new(None),
		};

		#[cfg(feature = "postgres")]
		if matches!(tx.inner, TxInner::Postgres(_))
			&& let Some(actor) = tx.access.actor()
		{
			tx.execute(Statement {
				sql: format!(
					"SELECT set_config('{}', $1, true)",
					leptoskit_db_schema::live::ACTOR_SETTING
				),
				args: vec![Value::Text(actor.uuid().to_string())],
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

		self.started(inner).await
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

	/// Applies `migration` to a development database without recording it as
	/// a migration, and records `schema`, a `schema.json` snapshot, as the
	/// schema the database now has. For `leptoskit dev`'s push mode: once a
	/// database has pushes, [`Db::migrate`] refuses to apply a migration on
	/// top of them until [`Db::adopt`] turns them into one.
	#[doc(hidden)]
	pub async fn push(
		&self,
		migration: &Migration,
		schema: &str,
	) -> std::result::Result<(), MigrationError> {
		match &*self.pool {
			#[cfg(feature = "postgres")]
			Pool::Postgres(pool) => pool.push(migration, schema).await,
			#[cfg(feature = "sqlite")]
			Pool::Sqlite(pool) => pool.push(migration.clone(), schema.to_string()).await,
		}
	}

	/// Records `migration` as applied without running it, since the
	/// database's pushes already made its changes, and forgets the pushes,
	/// in one transaction. Fails, changing nothing, when there are none.
	#[doc(hidden)]
	pub async fn adopt(&self, migration: &Migration) -> std::result::Result<(), MigrationError> {
		match &*self.pool {
			#[cfg(feature = "postgres")]
			Pool::Postgres(pool) => pool.adopt(migration).await,
			#[cfg(feature = "sqlite")]
			Pool::Sqlite(pool) => pool.adopt(migration.clone()).await,
		}
	}

	/// The `schema.json` snapshot of what pushes left this database with,
	/// when it has pushes no migration records yet.
	#[doc(hidden)]
	pub async fn pushed_schema(&self) -> Result<Option<String>> {
		if !self.has_table(crate::migrate::PUSH_TABLE).await? {
			return Ok(None);
		}

		let rows = self
			.fetch(Statement {
				sql: "SELECT \"schema\" FROM \"_leptoskit_push\"".to_owned(),
				args: Vec::new(),
			})
			.await?;

		Ok(rows.into_iter().find_map(|row| {
			match row.into_iter().next() {
				Some(Value::Text(schema)) => Some(schema),
				_ => None,
			}
		}))
	}

	/// Whether the database has a table named `name`, outside any
	/// namespace.
	async fn has_table(&self, name: &str) -> Result<bool> {
		let exists = match self.dialect() {
			Dialect::Postgres => {
				"SELECT count(*) FROM pg_catalog.pg_tables WHERE schemaname = current_schema() AND \
				 tablename = $1"
			}
			Dialect::Sqlite => {
				"SELECT count(*) FROM sqlite_schema WHERE type = 'table' AND name = ?1"
			}
		};

		let rows = self
			.fetch(Statement {
				sql: exists.to_owned(),
				args: vec![Value::Text(name.to_owned())],
			})
			.await?;
		Ok(
			matches!(rows.first().and_then(|row| row.first()), Some(Value::Int(count)) if *count > 0),
		)
	}

	/// The migrations this database has had, in the order they were applied.
	/// Reads only: a database that has never migrated has none.
	pub async fn applied_migrations(&self) -> Result<Vec<String>> {
		if !self.has_table(crate::MIGRATIONS_TABLE).await? {
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

	/// The Postgres URL the pool connects to, which the connection that
	/// listens for live changes uses too.
	#[cfg(feature = "postgres")]
	pub(crate) fn postgres_url(&self) -> Option<&str> {
		match &*self.pool {
			Pool::Postgres(pool) => Some(pool.url()),
			#[cfg(feature = "sqlite")]
			Pool::Sqlite(_) => None,
		}
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
	#[cfg_attr(
		not(feature = "sqlite"),
		expect(unused_variables, reason = "SQLite's pool wakes live readers itself")
	)]
	fn open(url: &str, options: PoolOptions, live: &Arc<Hub>) -> Result<Self> {
		#[cfg(feature = "postgres")]
		if url.starts_with("postgres://") || url.starts_with("postgresql://") {
			return crate::postgres::Pool::connect(url, options).map(Self::Postgres);
		}

		#[cfg(feature = "sqlite")]
		if url.starts_with("sqlite:") {
			return crate::sqlite::Pool::open(url, options, Arc::clone(live)).map(Self::Sqlite);
		}

		let scheme = url.split(':').next().unwrap_or_default().to_string();
		Err(DbError::connection(UnsupportedUrl(scheme)))
	}
}

/// Row-level security asked of a database without it.
#[derive(Debug, thiserror::Error)]
#[error(
	"only Postgres has row-level security; on SQLite the database layer alone enforces policies"
)]
struct NoRowSecurity;

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

	fn row_security(&self) -> bool {
		self.row_security
	}

	fn dialect(&self) -> Dialect {
		match &*self.pool {
			#[cfg(feature = "postgres")]
			Pool::Postgres(_) => Dialect::Postgres,
			#[cfg(feature = "sqlite")]
			Pool::Sqlite(_) => Dialect::Sqlite,
		}
	}

	async fn fetch(&self, statement: Statement) -> Result<Vec<Vec<Value>>> {
		match &*self.pool {
			#[cfg(feature = "postgres")]
			Pool::Postgres(pool) => {
				match self.row_settings()? {
					Some(settings) => pool.fetch_secured(&settings, statement).await,
					None => pool.fetch(statement).await,
				}
			}
			#[cfg(feature = "sqlite")]
			Pool::Sqlite(pool) => pool.fetch(statement).await,
		}
	}
}

impl Connection for Db {
	async fn execute(&self, statement: Statement) -> Result<u64> {
		match &*self.pool {
			#[cfg(feature = "postgres")]
			Pool::Postgres(pool) => {
				match self.row_settings()? {
					Some(settings) => pool.execute_secured(&settings, statement).await,
					None => pool.execute(statement).await,
				}
			}
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
	row_security: bool,
	/// SQLite: what the change log records the commit as made by, which
	/// Postgres keeps as a setting.
	#[cfg(feature = "sqlite")]
	origin: Mutex<Option<String>>,
}

enum TxInner {
	#[cfg(feature = "postgres")]
	Postgres(crate::postgres::Transaction),
	#[cfg(feature = "sqlite")]
	Sqlite(crate::sqlite::Transaction),
}

impl Tx {
	/// Makes every write permanent. When it wrote a live table, the commit
	/// takes the next version of the change log, and live subscribers hear
	/// of it.
	pub async fn commit(self) -> Result<()> {
		match self.inner {
			#[cfg(feature = "postgres")]
			TxInner::Postgres(tx) => tx.finish("COMMIT").await,
			#[cfg(feature = "sqlite")]
			TxInner::Sqlite(tx) => {
				let origin = self
					.origin
					.into_inner()
					.unwrap_or_else(std::sync::PoisonError::into_inner);
				tx.commit(self.access.actor().map(|actor| actor.uuid()), origin)
					.await
					.map(drop)
			}
		}
	}

	/// [`Tx::commit`], returning the change log version the commit took:
	/// the version a live subscriber has to reach to see this
	/// transaction's writes. `None` when it wrote no live table.
	///
	/// ```ignore
	/// let tx = db.begin().await?;
	/// NewMessage { body }.insert(&tx).await?;
	/// let version = tx.commit_version().await?; // Some(42)
	/// ```
	pub async fn commit_version(self) -> Result<Option<i64>> {
		match self.inner {
			#[cfg(feature = "postgres")]
			TxInner::Postgres(tx) => tx.commit_version().await,
			#[cfg(feature = "sqlite")]
			TxInner::Sqlite(tx) => {
				let origin = self
					.origin
					.into_inner()
					.unwrap_or_else(std::sync::PoisonError::into_inner);
				tx.commit(self.access.actor().map(|actor| actor.uuid()), origin)
					.await
			}
		}
	}

	/// Records what made this transaction's writes in the change log, such
	/// as the name of the operation that runs it. Without one, the commit's
	/// origin is `sql`.
	pub async fn set_origin(&self, origin: &str) -> Result<()> {
		match &self.inner {
			#[cfg(feature = "postgres")]
			TxInner::Postgres(tx) => {
				tx.execute(Statement {
					sql: format!(
						"SELECT set_config('{}', $1, true)",
						leptoskit_db_schema::live::ORIGIN_SETTING
					),
					args: vec![Value::Text(origin.to_string())],
				})
				.await
				.map(drop)
			}
			#[cfg(feature = "sqlite")]
			TxInner::Sqlite(_) => {
				*self
					.origin
					.lock()
					.unwrap_or_else(std::sync::PoisonError::into_inner) = Some(origin.to_string());
				Ok(())
			}
		}
	}

	/// Discards every write.
	pub async fn rollback(self) -> Result<()> {
		match self.inner {
			#[cfg(feature = "postgres")]
			TxInner::Postgres(tx) => tx.finish("ROLLBACK").await,
			#[cfg(feature = "sqlite")]
			TxInner::Sqlite(tx) => tx.rollback().await,
		}
	}
}

impl Tx {
	/// The connected database's dialect.
	pub fn dialect(&self) -> Dialect {
		Read::dialect(self)
	}
}

impl Read for Tx {
	fn access(&self) -> &Access {
		&self.access
	}

	fn row_security(&self) -> bool {
		self.row_security
	}

	fn dialect(&self) -> Dialect {
		match &self.inner {
			#[cfg(feature = "postgres")]
			TxInner::Postgres(_) => Dialect::Postgres,
			#[cfg(feature = "sqlite")]
			TxInner::Sqlite(_) => Dialect::Sqlite,
		}
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
