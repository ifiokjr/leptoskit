//! Where queries run.

use std::future::Future;

use leptoskit_db_schema::Dialect;

use crate::Result;
use crate::Value;
use crate::sql::Statement;

/// Somewhere queries run: the pool (`Db`) or a transaction (`Tx`).
///
/// Every query and write takes `&impl Connection`, so the same code runs
/// inside or outside a transaction.
pub trait Connection: Send + Sync {
	/// The connected database's dialect.
	fn dialect(&self) -> Dialect;

	/// Runs a statement and returns its rows, each as values in column order.
	#[doc(hidden)]
	fn fetch(&self, statement: Statement) -> impl Future<Output = Result<Vec<Vec<Value>>>> + Send;

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
