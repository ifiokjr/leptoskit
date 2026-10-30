//! The connection pool, and transactions within it. Needs a driver: the
//! `postgres` or `sqlite` feature.

use std::sync::Arc;
use std::sync::Mutex;

use leptoskit_db_schema::Dialect;

use crate::Connection;
use crate::DbError;
use crate::Migration;
use crate::MigrationError;
use crate::PoolOptions;
use crate::Result;
use crate::Value;
#[cfg(feature = "sqlite")]
use crate::live::Event;
use crate::live::Hub;
use crate::live::RowChange;
use crate::policy::Access;
use crate::policy::Context;
use crate::sql::Statement;

/// The app's database: a cheap-to-clone handle to a connection pool.
#[derive(Clone)]
pub struct Db {
	pool: Arc<Pool>,
	pub(crate) live: Arc<Hub>,
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
			live: Arc::new(Hub::new()),
			access: Access::default(),
		})
	}

	/// The same pool, acting for a request: policies read `cx`. Build one
	/// per request once its session is known.
	#[must_use]
	pub fn with_context(&self, cx: Context) -> Self {
		Self {
			pool: Arc::clone(&self.pool),
			live: Arc::clone(&self.live),
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
			live: Arc::clone(&self.live),
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

		Ok(Tx {
			inner,
			access: self.access.clone(),
			#[cfg(feature = "sqlite")]
			live: Arc::clone(&self.live),
			pending: Mutex::new(Vec::new()),
		})
	}

	/// The changes to `#[db::table(live)]` tables this handle may know
	/// about, from now on: each committed insert, update, and delete whose
	/// row this handle's `select` policy allows, before or after the change.
	/// Make one per subscriber, from the request's handle.
	///
	/// ```ignore
	/// let mut changes = request_db.changes().await?;
	/// while let Ok(invalidation) = changes.next().await {
	/// 	events.send(invalidation).await?;
	/// }
	/// ```
	///
	/// On Postgres the first call opens a connection that listens for every
	/// server's changes, and returns once it's listening.
	#[cfg_attr(
		not(feature = "postgres"),
		expect(
			clippy::unused_async,
			reason = "only Postgres listens before subscribing"
		)
	)]
	pub async fn changes(&self) -> Result<crate::live::Feed> {
		match &*self.pool {
			#[cfg(feature = "postgres")]
			Pool::Postgres(pool) => {
				self.live
					.listener
					.get_or_try_init(|| {
						crate::postgres::Listener::start(pool.url(), Arc::downgrade(&self.live))
					})
					.await?;
			}
			// Commits in this process are all there is to hear.
			#[cfg(feature = "sqlite")]
			Pool::Sqlite(_) => {}
		}

		Ok(crate::live::Feed::new(self.clone(), self.live.subscribe()))
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

impl Connection for Db {
	fn access(&self) -> &Access {
		&self.access
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
			Pool::Postgres(pool) => pool.fetch(statement).await,
			#[cfg(feature = "sqlite")]
			Pool::Sqlite(pool) => pool.fetch(statement).await,
		}
	}

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

	async fn publish(&self, changes: Vec<RowChange>) -> Result<()> {
		match &*self.pool {
			#[cfg(feature = "postgres")]
			Pool::Postgres(_) => {
				for statement in notifications(&changes)? {
					self.execute(statement).await?;
				}
			}
			#[cfg(feature = "sqlite")]
			Pool::Sqlite(_) => self.live.send(Event::Changes(changes.into())),
		}

		Ok(())
	}
}

/// `SELECT pg_notify(…)` for each part of a change set.
#[cfg(feature = "postgres")]
fn notifications(changes: &[RowChange]) -> Result<Vec<Statement>> {
	Ok(crate::live::encode(changes)?
		.into_iter()
		.map(|payload| {
			Statement {
				sql: "SELECT pg_notify($1, $2)".to_string(),
				args: vec![
					Value::Text(crate::live::CHANNEL.to_string()),
					Value::Text(payload),
				],
			}
		})
		.collect())
}

/// A transaction. Queries through it see each other's writes, and nothing
/// is visible to anyone else until it commits.
pub struct Tx {
	inner: TxInner,
	access: Access,
	/// Where SQLite's commits tell live subscribers; Postgres tells them
	/// itself.
	#[cfg(feature = "sqlite")]
	live: Arc<Hub>,
	/// Live changes, published when the transaction commits.
	pending: Mutex<Vec<RowChange>>,
}

enum TxInner {
	#[cfg(feature = "postgres")]
	Postgres(crate::postgres::Transaction),
	#[cfg(feature = "sqlite")]
	Sqlite(crate::sqlite::Transaction),
}

impl Tx {
	/// Makes every write permanent, and tells live subscribers what it
	/// changed.
	pub async fn commit(self) -> Result<()> {
		let pending = std::mem::take(&mut *lock(&self.pending));

		match self.inner {
			#[cfg(feature = "postgres")]
			TxInner::Postgres(tx) => {
				// Postgres delivers a transaction's notifications when, and
				// only if, it commits.
				if !pending.is_empty() {
					for statement in notifications(&pending)? {
						tx.execute(statement).await?;
					}
				}

				tx.finish("COMMIT").await
			}
			#[cfg(feature = "sqlite")]
			TxInner::Sqlite(tx) => {
				tx.finish("COMMIT").await?;

				if !pending.is_empty() {
					self.live.send(Event::Changes(pending.into()));
				}

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
			TxInner::Sqlite(tx) => tx.finish("ROLLBACK").await,
		}
	}
}

impl Connection for Tx {
	fn access(&self) -> &Access {
		&self.access
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

	async fn publish(&self, changes: Vec<RowChange>) -> Result<()> {
		lock(&self.pending).extend(changes);
		Ok(())
	}
}

/// The pending changes, even after a panic elsewhere poisoned the lock:
/// they're only ever appended to.
fn lock(pending: &Mutex<Vec<RowChange>>) -> std::sync::MutexGuard<'_, Vec<RowChange>> {
	pending
		.lock()
		.unwrap_or_else(std::sync::PoisonError::into_inner)
}
