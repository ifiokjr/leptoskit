//! The connection pool, and transactions within it. Needs a driver: the
//! `postgres` or `sqlite` feature.

use std::sync::Arc;

use leptoskit_db_schema::Dialect;

use crate::Connection;
use crate::DbError;
use crate::PoolOptions;
use crate::Result;
use crate::Value;
use crate::sql::Statement;

/// The app's database: a cheap-to-clone handle to a connection pool.
#[derive(Clone)]
pub struct Db {
	pool: Arc<Pool>,
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
	pub async fn connect_with(url: &str, options: PoolOptions) -> Result<Self> {
		let pool = Pool::open(url, options).await?;
		Ok(Self {
			pool: Arc::new(pool),
		})
	}

	/// Runs `work` in a transaction: it commits when `work` returns `Ok`, and
	/// rolls back when it returns `Err` or is dropped part-way, such as when
	/// the request is cancelled.
	///
	/// ```ignore
	/// let post = db.transaction(async |tx| {
	/// 	let post = NewPost { … }.insert(tx).await?;
	/// 	NewTagging { post_id: post.id, … }.insert(tx).await?;
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

		Ok(Tx { inner })
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
		not(feature = "postgres"),
		expect(
			clippy::unused_async,
			reason = "only the Postgres pool connects asynchronously"
		)
	)]
	async fn open(url: &str, options: PoolOptions) -> Result<Self> {
		#[cfg(feature = "postgres")]
		if url.starts_with("postgres://") || url.starts_with("postgresql://") {
			return crate::postgres::Pool::connect(url, options)
				.await
				.map(Self::Postgres);
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
}

/// A transaction. Queries through it see each other's writes, and nothing
/// is visible to anyone else until it commits.
pub struct Tx {
	inner: TxInner,
}

enum TxInner {
	#[cfg(feature = "postgres")]
	Postgres(crate::postgres::Transaction),
	#[cfg(feature = "sqlite")]
	Sqlite(crate::sqlite::Transaction),
}

impl Tx {
	/// Makes every write permanent.
	pub async fn commit(self) -> Result<()> {
		match self.inner {
			#[cfg(feature = "postgres")]
			TxInner::Postgres(tx) => tx.finish("COMMIT").await,
			#[cfg(feature = "sqlite")]
			TxInner::Sqlite(tx) => tx.finish("COMMIT").await,
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
}
