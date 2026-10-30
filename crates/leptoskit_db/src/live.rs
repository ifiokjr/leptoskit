//! Change capture: the pack's tables, and the feed over them.
//!
//! Every write to a `#[db::table(live)]` table inserts a row into
//! `live.changes` in the writing transaction. A transaction that wrote one
//! gets the next version in `live.commits`, allocated at commit on Postgres
//! by a deferred constraint trigger, and just before commit on SQLite by the
//! transaction that wrote it.
//!
//! [`Db::changes`] reads the captured changes in version order. The RPC's
//! live hub is its only caller today: it matches each change against the
//! queries subscribed to the table, re-runs the ones it may affect, and sends
//! the results. Nothing sends rows from this feed to a client.

use std::time::Duration;

use leptoskit_db_schema::Dialect;
use leptoskit_db_schema::TableName;
use leptoskit_db_schema::Trigger;
use serde_json::Value as JsonValue;
use uuid::Uuid;

use crate::Connection;
use crate::Db;
use crate::Enum;
use crate::Json;
use crate::Read;
use crate::Result;
use crate::Table;
use crate::Timestamp;
use crate::Value;
use crate::table;

/// What a change did to a row.
#[derive(Enum, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ChangeOp {
	/// The row was inserted.
	#[db(rename = "I")]
	Insert,
	/// The row was updated.
	#[db(rename = "U")]
	Update,
	/// The row was deleted.
	#[db(rename = "D")]
	Delete,
}

/// One committed transaction that wrote a live table.
#[table(name = "commits", namespace = "live", __internal, unique(txid))]
pub struct Commit {
	/// The transaction's place in the version line. Gaps are rolled-back
	/// transactions, which never become visible.
	#[db(key)]
	pub version: i64,
	/// The Postgres transaction ID, which joins the commit to its changes.
	/// `None` on SQLite.
	pub txid: Option<i64>,
	/// When the transaction committed.
	pub committed_at: Timestamp,
	/// Who made the change, when someone was acting.
	pub actor: Option<Uuid>,
	/// What made it: an operation's name, or `sql`.
	pub origin: String,
}

/// One written row of a live table.
#[table(
	name = "changes",
	namespace = "live",
	__internal,
	index(txid),
	index(version)
)]
pub struct Change {
	/// The change's position within the database.
	#[db(key)]
	pub seq: i64,
	/// The Postgres transaction that wrote it, which joins it to its commit.
	pub txid: Option<i64>,
	/// The commit's version. `None` until the transaction commits.
	pub version: Option<i64>,
	/// The changed table, with its namespace: `messages` or `billing.invoices`.
	pub table_name: String,
	/// What the write did.
	pub op: ChangeOp,
	/// The changed row's key, as the capture trigger recorded it.
	pub key: Json<Vec<JsonValue>>,
	/// The row before an update or delete, without `secret` and
	/// `live = skip` columns.
	pub old: Option<Json<JsonValue>>,
	/// The row after an insert or update, without those columns.
	pub new: Option<Json<JsonValue>>,
	/// For an update, the columns whose values changed.
	pub changed: Json<Vec<String>>,
	/// When the change was made, copied from the commit for retention.
	pub committed_at: Option<Timestamp>,
}

/// Postgres only: the transactions waiting for a version at commit.
#[table(name = "pending", namespace = "live", __internal)]
pub struct Pending {
	/// The transaction's ID.
	#[db(key)]
	pub txid: i64,
}

impl Change {
	/// The table's name, as the capture trigger wrote it.
	pub fn table(&self) -> &str {
		&self.table_name
	}
}

/// The pack's tables for `dialect`: commits and changes on both, and the
/// pending transactions that get a version at commit on Postgres.
pub(crate) fn tables(dialect: Dialect) -> Vec<leptoskit_db_schema::Table> {
	let mut tables = vec![Commit::definition(), Change::definition()];

	if dialect == Dialect::Postgres {
		tables.push(Pending::definition());
	}

	tables
}

/// The capture triggers for the live tables in `entries`.
pub(crate) fn capture_triggers(
	dialect: Dialect,
	tables: &[(leptoskit_db_schema::Table, &'static [&'static str])],
) -> Vec<Trigger> {
	tables
		.iter()
		.flat_map(|(table, columns)| leptoskit_db_schema::live::capture(dialect, table, columns))
		.collect()
}

/// A read of the change log at or after a version.
///
/// ```ignore
/// let changes = db.changes().since(version).all().await?;
/// ```
pub struct Changes<'a> {
	db: &'a Db,
	since: i64,
	limit: Option<u64>,
}

impl<'a> Changes<'a> {
	/// The changes committed after `version`.
	#[must_use]
	pub fn since(mut self, version: i64) -> Self {
		self.since = version;
		self
	}

	/// At most `limit` changes, oldest first.
	#[must_use]
	pub fn limit(mut self, limit: u64) -> Self {
		self.limit = Some(limit);
		self
	}

	/// Every change since the version, in commit order.
	pub async fn all(self) -> Result<Vec<Change>> {
		let table = TableName::namespaced("live", "changes");
		let commits = TableName::namespaced("live", "commits");
		let (sql, args) = match self.db.dialect() {
			Dialect::Postgres => {
				(
					format!(
						"SELECT ch.seq, ch.txid, c.version, ch.table_name, ch.op, ch.key, ch.old, \
						 ch.new, ch.changed, c.committed_at FROM {} ch JOIN {} c ON c.txid = \
						 ch.txid WHERE c.version > $1 ORDER BY c.version, ch.seq{}",
						table_name(self.db.dialect(), &table),
						table_name(self.db.dialect(), &commits),
						self.limit
							.map(|limit| format!(" LIMIT {limit}"))
							.unwrap_or_default()
					),
					vec![Value::Int(self.since)],
				)
			}
			Dialect::Sqlite => {
				(
					format!(
						"SELECT seq, txid, version, table_name, op, key, old, new, changed, \
						 committed_at FROM {} WHERE version > ?1 ORDER BY version, seq{}",
						table_name(self.db.dialect(), &table),
						self.limit
							.map(|limit| format!(" LIMIT {limit}"))
							.unwrap_or_default()
					),
					vec![Value::Int(self.since)],
				)
			}
		};
		let rows = self.db.fetch(crate::sql::Statement { sql, args }).await?;
		rows.into_iter()
			.map(|row| Change::from_values(row).map_err(|error| crate::DbError::Decode(error)))
			.collect()
	}
}

/// The version of the newest commit, or 0 when nothing has committed.
pub async fn latest_version(db: &Db) -> Result<i64> {
	let (sql, _) = match db.dialect() {
		Dialect::Postgres => {
			(
				format!(
					"SELECT coalesce(max(version), 0) FROM {}",
					table_name(db.dialect(), &TableName::namespaced("live", "commits"))
				),
				(),
			)
		}
		Dialect::Sqlite => {
			(
				format!(
					"SELECT coalesce(max(version), 0) FROM {}",
					table_name(db.dialect(), &TableName::namespaced("live", "commits"))
				),
				(),
			)
		}
	};
	let rows = db
		.fetch(crate::sql::Statement {
			sql,
			args: Vec::new(),
		})
		.await?;

	rows.first()
		.and_then(|row| row.first())
		.and_then(|value| {
			match value {
				Value::Int(version) => Some(*version),
				_ => None,
			}
		})
		.ok_or_else(|| {
			crate::DbError::Decode(crate::DecodeError::custom(
				"the live version query returned no integer",
			))
		})
}

/// Deletes commits, changes, and their changed rows older than `retention`.
///
/// A client that reconnects after the window re-runs its queries from
/// scratch; resume is an optimisation, so deleting old rows is always safe.
/// Batched, so a large backlog never holds a long transaction.
pub async fn prune(db: &Db, retention: Duration) -> Result<u64> {
	let cutoff = crate::now()
		- chrono::Duration::from_std(retention).unwrap_or_else(|_| chrono::Duration::zero());
	let mut removed = 0;
	let batch = 5_000;
	let changes = TableName::namespaced("live", "changes");
	let commits = TableName::namespaced("live", "commits");

	loop {
		let (delete, args) = match db.dialect() {
			Dialect::Postgres => {
				(
					format!(
						"DELETE FROM {} WHERE seq IN (SELECT ch.seq FROM {} ch JOIN {} c ON \
						 c.txid = ch.txid WHERE c.committed_at <= $1 LIMIT {batch})",
						table_name(db.dialect(), &changes),
						table_name(db.dialect(), &changes),
						table_name(db.dialect(), &commits)
					),
					vec![Value::Timestamp(cutoff)],
				)
			}
			Dialect::Sqlite => {
				(
					format!(
						"DELETE FROM {} WHERE seq IN (SELECT seq FROM {} WHERE committed_at <= $1 \
						 LIMIT {batch})",
						table_name(db.dialect(), &changes),
						table_name(db.dialect(), &changes)
					),
					vec![Value::Timestamp(cutoff)],
				)
			}
		};
		let deleted = db
			.execute(crate::sql::Statement { sql: delete, args })
			.await?;
		removed += deleted;

		if deleted < batch {
			break;
		}
	}

	let (delete, args) = match db.dialect() {
		Dialect::Postgres => {
			(
				format!(
					"DELETE FROM {} WHERE committed_at <= $1 AND NOT EXISTS (SELECT 1 FROM {} ch \
					 WHERE ch.txid = {}.txid)",
					table_name(db.dialect(), &commits),
					table_name(db.dialect(), &changes),
					table_name(db.dialect(), &commits)
				),
				vec![Value::Timestamp(cutoff)],
			)
		}
		Dialect::Sqlite => {
			(
				format!(
					"DELETE FROM {} WHERE committed_at <= $1",
					table_name(db.dialect(), &commits)
				),
				vec![Value::Timestamp(cutoff)],
			)
		}
	};
	removed += db
		.execute(crate::sql::Statement { sql: delete, args })
		.await?;

	Ok(removed)
}

/// A table's name in the database's own SQL: namespaced on Postgres, flat on
/// SQLite.
fn table_name(dialect: Dialect, table: &TableName) -> String {
	match (dialect, table.namespace.as_deref()) {
		(Dialect::Postgres, Some(namespace)) => {
			format!(
				"{}.{}",
				leptoskit_db_schema::quote_identifier(namespace),
				leptoskit_db_schema::quote_identifier(&table.name)
			)
		}
		_ => leptoskit_db_schema::quote_identifier(&table.flat()),
	}
}

/// On SQLite, stamps the version on every change this transaction wrote and
/// records the commit, just before `COMMIT`. Does nothing in a database
/// without the live pack.
#[cfg(feature = "sqlite")]
pub(crate) fn seal_sqlite(
	connection: &rusqlite::Connection,
	actor: Option<Uuid>,
	origin: &str,
) -> rusqlite::Result<()> {
	let has_changes: bool = connection.query_row(
		"SELECT count(*) FROM sqlite_schema WHERE type = 'table' AND name = 'live_changes'",
		[],
		|row| row.get::<_, i64>(0),
	)? > 0;

	if !has_changes {
		return Ok(());
	}

	let actor = actor.map_or_else(
		|| "NULL".to_string(),
		|actor| format!("X'{}'", actor.simple()),
	);
	connection.execute_batch(&format!(
		"UPDATE live_changes SET version = (SELECT coalesce(max(version), 0) + 1 FROM \
		 live_commits), committed_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE version IS \
		 NULL;
		 INSERT INTO live_commits (version, txid, committed_at, actor, origin)
		 SELECT DISTINCT version, NULL, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), {}, {} FROM live_changes \
		 WHERE version IS NOT NULL AND version NOT IN (SELECT version FROM live_commits);",
		actor,
		sql_literal(Some(origin)),
	))?;
	Ok(())
}

#[cfg(feature = "sqlite")]
fn sql_literal(value: Option<&str>) -> String {
	value.map_or_else(
		|| "NULL".to_string(),
		|value| format!("'{}'", value.replace('\'', "''")),
	)
}

#[cfg(any(feature = "postgres", feature = "sqlite"))]
impl Db {
	/// Reads the change log. The live hub uses it to find what a commit
	/// changed; the RPC turns those changes into new query results.
	pub fn changes(&self) -> Changes<'_> {
		Changes {
			db: self,
			since: 0,
			limit: None,
		}
	}
}
