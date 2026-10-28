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
