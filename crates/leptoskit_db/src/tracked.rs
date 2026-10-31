//! A compatibility layer for operation contexts while the database's
//! read-only split is rebuilt on the live-invalidation model.
//!
//! `Tracked` wraps a connection and exposes the read-set shape the RPC's
//! query context expects. It does not record reads yet: live subscriptions
//! currently use the invalidation feed from
//! [`Db::changes`](crate::Db::changes).

use std::future::Future;

use leptoskit_db_schema::Dialect;

use crate::Connection;
use crate::Result;
use crate::Value;
use crate::policy::Access;
use crate::sql::Statement;

/// The read half of a connection. Every [`Connection`] is one.
pub trait Read: Connection {}

impl<T: Connection> Read for T {}

/// One thing a tracked read read. Reserved for the read-set matcher; the
/// invalidation feed does not need it yet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReadEntry;

/// A connection that will record what a live query reads.
pub struct Tracked<C> {
	connection: C,
}

impl<C: Connection> Tracked<C> {
	/// Wraps `connection`.
	pub fn new(connection: C) -> Self {
		Self { connection }
	}

	/// Everything read so far. Empty until read sets return.
	pub fn reads(&self) -> Vec<ReadEntry> {
		Vec::new()
	}

	/// Everything read, ending the tracking.
	pub fn into_reads(self) -> Vec<ReadEntry> {
		Vec::new()
	}
}

impl<C: Connection> Connection for Tracked<C> {
	fn dialect(&self) -> Dialect {
		self.connection.dialect()
	}

	fn access(&self) -> &Access {
		self.connection.access()
	}

	fn row_security(&self) -> bool {
		self.connection.row_security()
	}

	fn fetch(&self, statement: Statement) -> impl Future<Output = Result<Vec<Vec<Value>>>> + Send {
		self.connection.fetch(statement)
	}

	fn execute(&self, statement: Statement) -> impl Future<Output = Result<u64>> + Send {
		self.connection.execute(statement)
	}

	fn publish(
		&self,
		changes: Vec<crate::live::RowChange>,
	) -> impl Future<Output = Result<()>> + Send {
		self.connection.publish(changes)
	}

	#[cfg(any(feature = "postgres", feature = "sqlite"))]
	fn begin_unless_in_transaction(
		&self,
	) -> impl Future<Output = Result<Option<crate::Tx>>> + Send {
		self.connection.begin_unless_in_transaction()
	}
}
