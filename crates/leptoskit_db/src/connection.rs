//! Where queries run.

use std::future::Future;

use leptoskit_db_schema::Dialect;

use crate::Result;
use crate::Value;
use crate::sql::Statement;

/// Somewhere queries read: the pool (`Db`), a transaction (`Tx`), or a
/// [`Tracked`](crate::Tracked) connection that records what it reads.
///
/// Every read takes `&impl Read`: [`Query::all`](crate::Query::all) and its
/// relatives, `get` and `find`, link accessors, shapes, and views. A
/// connection that only implements `Read` can't write, so code given one is
/// read-only by its type.
pub trait Read: Send + Sync {
	/// The connected database's dialect.
	fn dialect(&self) -> Dialect;

	/// Who the connection acts for, which decides the policies it obeys.
	#[doc(hidden)]
	fn access(&self) -> &crate::policy::Access;

	/// Runs a statement and returns its rows, each as values in column order,
	/// without saying what it reads. A connection that records its reads
	/// refuses it: raw SQL declares its tables with
	/// [`Statement::reads`](crate::Statement::reads).
	#[doc(hidden)]
	fn fetch(&self, statement: Statement) -> impl Future<Output = Result<Vec<Vec<Value>>>> + Send;

	/// Runs a read the database layer built, which `describe` says what it
	/// reads. Only a connection that records its reads calls `describe`.
	#[doc(hidden)]
	fn read(
		&self,
		statement: Statement,
		describe: impl FnOnce() -> Result<crate::read_set::Reads> + Send,
	) -> impl Future<Output = Result<Vec<Vec<Value>>>> + Send {
		let _ = describe;
		self.fetch(statement)
	}
}

impl<C: Read> Read for &C {
	fn dialect(&self) -> Dialect {
		(**self).dialect()
	}

	fn access(&self) -> &crate::policy::Access {
		(**self).access()
	}

	fn fetch(&self, statement: Statement) -> impl Future<Output = Result<Vec<Vec<Value>>>> + Send {
		(**self).fetch(statement)
	}

	fn read(
		&self,
		statement: Statement,
		describe: impl FnOnce() -> Result<crate::read_set::Reads> + Send,
	) -> impl Future<Output = Result<Vec<Vec<Value>>>> + Send {
		(**self).read(statement, describe)
	}
}

/// Somewhere queries run and rows are written: the pool (`Db`) or a
/// transaction (`Tx`).
///
/// Every write takes `&impl Connection`, so the same code runs inside or
/// outside a transaction. Reads take anything that implements [`Read`],
/// which every connection does.
#[diagnostic::on_unimplemented(
	message = "`{Self}` only reads, so it can't write",
	label = "a read-only connection",
	note = "writes take a `Connection`, such as a `Db` or a `Tx`; a `Tracked` connection records \
	        what a live query reads, and never writes"
)]
pub trait Connection: Read {
	/// Runs a statement and returns how many rows it changed.
	#[doc(hidden)]
	fn execute(&self, statement: Statement) -> impl Future<Output = Result<u64>> + Send;

	/// A transaction for work that must happen all at once, or `None` when
	/// this already is one and the work joins it.
	#[doc(hidden)]
	#[cfg(any(feature = "postgres", feature = "sqlite"))]
	fn begin_unless_in_transaction(&self)
	-> impl Future<Output = Result<Option<crate::Tx>>> + Send;
}

/// Runs `$work` all at once: in a new transaction on a pool, or as part of
/// the transaction `$connection` already is. `$tx` names the connection the
/// work runs on.
#[cfg(any(feature = "postgres", feature = "sqlite"))]
macro_rules! atomically {
	($connection:expr, | $tx:ident | $work:expr) => {
		match $connection.begin_unless_in_transaction().await? {
			Some(transaction) => {
				let result = {
					let $tx = &transaction;
					$work
				};
				crate::connection::finish(transaction, result).await
			}
			None => {
				let $tx = $connection;
				$work
			}
		}
	};
}

/// Without a driver there is no connection to run anything on, so the work
/// runs as it is.
#[cfg(not(any(feature = "postgres", feature = "sqlite")))]
macro_rules! atomically {
	($connection:expr, | $tx:ident | $work:expr) => {{
		let $tx = $connection;
		$work
	}};
}

pub(crate) use atomically;

/// Runs `result`'s transaction to its end: commits when the work succeeded,
/// and rolls back when it failed.
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub(crate) async fn finish<T>(tx: crate::Tx, result: Result<T>) -> Result<T> {
	match result {
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

/// How many connections a pool may hold.
#[derive(Clone, Copy, Debug)]
pub struct PoolOptions {
	/// The most connections open at once. An in-memory SQLite database always
	/// uses one, since each connection would otherwise see its own database.
	pub max_connections: usize,
}

impl Default for PoolOptions {
	fn default() -> Self {
		Self { max_connections: 8 }
	}
}
