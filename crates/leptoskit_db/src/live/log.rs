//! The change log: the `live` pack's tables, which the capture triggers that
//! migrations create fill in the writing transaction, and reading them back
//! in version order.
//!
//! Every write to a `#[db::table(live)]` table, from the database layer, raw
//! SQL, `psql`, or another service, inserts a row into `live.changes`. Each
//! committing transaction that wrote one gets the next version in
//! `live.commits`: on Postgres from `live.seal()`, a deferred constraint
//! trigger that runs at commit under a transaction-scoped advisory lock, so
//! versions become visible in order; on SQLite, which has one writer at a
//! time, from the pool just before it commits.

use leptoskit_db_schema::Dialect;
use leptoskit_db_schema::TableName;
use uuid::Uuid;

use crate::Enum;
use crate::Json;
use crate::Read;
use crate::Table;
use crate::Timestamp;
use crate::table;

/// What a write did to a row.
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

/// `live.commits`: one row per committed transaction that wrote a live
/// table.
#[table(name = "commits", namespace = "live", __internal, index(txid))]
pub struct LogCommit {
	/// The transaction's place in the version line. Gaps are transactions
	/// that rolled back after taking a version, which never become visible.
	#[db(key)]
	pub version: i64,
	/// The Postgres transaction ID, which joins the commit to its changes.
	/// `None` on SQLite.
	pub txid: Option<i64>,
	/// When the transaction committed.
	pub committed_at: Timestamp,
	/// Who made the change, when the transaction said.
	pub actor: Option<Uuid>,
	/// What made it: what the transaction said, such as an operation's
	/// name, or `sql`.
	pub origin: String,
	/// Replication capture: where the commit's record starts in Postgres's
	/// write-ahead log, which orders commits as the database committed them.
	/// `None` for commits captured by triggers.
	pub lsn: Option<i64>,
}

/// `live.changes`: one row per written row of a live table.
#[table(
	name = "changes",
	namespace = "live",
	__internal,
	index(txid),
	index(version)
)]
pub struct LogChange {
	/// The change's place among the database's changes.
	#[db(key)]
	pub seq: i64,
	/// The Postgres transaction that wrote it, which joins it to its commit.
	pub txid: Option<i64>,
	/// SQLite: the commit's version, set just before the commit.
	pub version: Option<i64>,
	/// The changed table, with its namespace: `messages` or `billing.invoices`.
	pub table_name: String,
	/// What the write did.
	pub op: ChangeOp,
	/// The changed row's key, in key column order.
	pub key: Json<Vec<serde_json::Value>>,
	/// The row before an update or delete, without `secret` and
	/// `live = skip` columns.
	pub old: Option<Json<serde_json::Value>>,
	/// The row after an insert or update, without those columns.
	pub new: Option<Json<serde_json::Value>>,
	/// For an update, every column whose value changed; empty otherwise.
	pub changed: Json<Vec<String>>,
}

/// `live.pending`, Postgres only: the transactions waiting for a version at
/// commit.
#[table(name = "pending", namespace = "live", __internal)]
pub struct LogPending {
	/// The transaction's ID.
	#[db(key)]
	pub txid: i64,
}

/// The pack's tables for `dialect`: commits and changes on both, and the
/// pending transactions that get a version at commit on Postgres.
///
/// On Postgres the commits are partitioned by version and the changes by
/// their order, both of which a row has when it's written, whichever way
/// it's captured: a trigger writes a change before its commit has a
/// version. Pruning then drops whole partitions rather than deleting rows.
pub(crate) fn tables(dialect: Dialect) -> Vec<leptoskit_db_schema::Table> {
	if dialect == Dialect::Postgres {
		return vec![
			LogCommit::definition().partitioned_by("version"),
			LogChange::definition().partitioned_by("seq"),
			LogPending::definition(),
		];
	}

	vec![LogCommit::definition(), LogChange::definition()]
}

/// One committed transaction's writes to live tables, read from the change
/// log: what [`Db::change_log`](crate::Db::change_log) yields.
///
/// It serializes, so a server can pass commits on to others, as the live
/// relay does.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Commit {
	/// Its place in the version line.
	pub version: i64,
	/// When it committed.
	pub committed_at: Timestamp,
	/// Who made it, when the transaction said: see
	/// [`Tx::set_origin`](crate::Tx::set_origin).
	pub actor: Option<Uuid>,
	/// What made it: an operation's name, or `sql`.
	pub origin: String,
	/// Each written row, in the order the transaction wrote them.
	pub changes: Vec<Change>,
	/// Postgres: a position in the write-ahead log at or past the end of
	/// this commit's record, so a read replica that has replayed up to it
	/// shows the commit. With replication capture it is just past the
	/// commit's own record; with triggers, which can't know where their
	/// commit lands, it is the primary's insert position when the log was
	/// read, which is later. `None` on SQLite.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub lsn: Option<i64>,
}

/// One written row of a live table.
///
/// Its images hold the row's columns except `secret` ones and those marked
/// `#[db(live = skip)]`, as JSON that reads the same on both databases: an
/// `Id` as its hyphenated UUID, a timestamp in RFC 3339, bytes as hex. A
/// table this build doesn't know keeps its images as the database wrote
/// them.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Change {
	/// The table, with its namespace.
	pub table: TableName,
	/// What the write did.
	pub op: ChangeOp,
	/// The row's key, one text per key column, as
	/// [`ChangedRows`](super::ChangedRows) names rows.
	pub key: Vec<String>,
	/// The row before an update or delete.
	pub old: Option<serde_json::Map<String, serde_json::Value>>,
	/// The row after an insert or update.
	pub new: Option<serde_json::Map<String, serde_json::Value>>,
	/// For an update, the columns whose values changed, `secret` and
	/// skipped ones included.
	pub changed: Vec<String>,
}

/// What reading the change log yields next.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogEntry {
	/// A committed transaction.
	Commit(Commit),
	/// The log was pruned past the reader's version: anything up to and
	/// including `through` may have been missed.
	Pruned {
		/// The newest version pruned.
		through: i64,
	},
}

/// One change as the log holds it, before its table is looked up.
#[cfg(any(feature = "postgres", feature = "sqlite"))]
#[derive(Clone, Debug)]
pub(crate) struct RawChange {
	pub(crate) table: TableName,
	pub(crate) op: ChangeOp,
	pub(crate) key: Vec<serde_json::Value>,
	pub(crate) old: Option<serde_json::Map<String, serde_json::Value>>,
	pub(crate) new: Option<serde_json::Map<String, serde_json::Value>>,
	pub(crate) changed: Vec<String>,
}

/// One commit as the log holds it.
#[cfg(any(feature = "postgres", feature = "sqlite"))]
#[derive(Clone, Debug)]
pub(crate) struct RawCommit {
	pub(crate) version: i64,
	pub(crate) committed_at: Timestamp,
	pub(crate) actor: Option<Uuid>,
	pub(crate) origin: String,
	pub(crate) changes: Vec<RawChange>,
	/// See [`Commit::lsn`].
	pub(crate) lsn: Option<i64>,
}

/// What a read of the log yields.
#[cfg(any(feature = "postgres", feature = "sqlite"))]
#[derive(Clone, Debug)]
pub(crate) enum RawEntry {
	Commit(RawCommit),
	Pruned { through: i64 },
}

#[cfg(any(feature = "postgres", feature = "sqlite"))]
mod reading {
	use std::collections::VecDeque;

	use super::*;
	use crate::Connection;
	use crate::Db;
	use crate::DbError;
	use crate::DecodeError;
	use crate::Result;
	use crate::Value;
	use crate::sql::Statement;

	/// The origin a pruned commit is left with: it marks how far back the
	/// log was emptied, so a reader that was further behind knows it missed
	/// changes. No operation is named this.
	pub(crate) const PRUNED: &str = "leptoskit.pruned";

	/// The most commits one read of the log returns.
	const COMMITS_PER_READ: usize = 256;

	/// Reads the change log after a version, waking when a commit may have
	/// added to it: what [`Feed`](super::super::Feed) and
	/// [`ChangeLog`](super::super::ChangeLog) are built on.
	pub(crate) struct Reader {
		db: Db,
		version: i64,
		wake: tokio::sync::watch::Receiver<u64>,
		ready: VecDeque<RawEntry>,
	}

	impl Reader {
		/// Starts reading after `since`, or after the newest commit when
		/// `None`. On Postgres the first reader opens the connection that
		/// listens for commits, and returns once it's listening.
		pub(crate) async fn start(db: Db, since: Option<i64>) -> Result<Self> {
			db.listen().await?;
			let wake = db.live.subscribe();

			let version = match since {
				Some(version) => version,
				None => latest_version(&db).await?,
			};

			Ok(Self {
				db,
				version,
				wake,
				ready: VecDeque::new(),
			})
		}

		/// The newest version this reader has passed.
		pub(crate) fn version(&self) -> i64 {
			self.version
		}

		pub(crate) fn db(&self) -> &Db {
			&self.db
		}

		/// The next entry an earlier read already fetched, without reading
		/// the log again: `None` when there's none to hand.
		pub(crate) fn next_ready(&mut self) -> Option<RawEntry> {
			let entry = self.ready.pop_front()?;
			self.version = match &entry {
				RawEntry::Commit(commit) => commit.version,
				RawEntry::Pruned { through } => *through,
			};
			Some(entry)
		}

		/// The next entry, waiting for one. The reader moves past it, so a
		/// later call reads on from there.
		///
		/// # Errors
		///
		/// When the log can't be read; the next call tries again from the
		/// same version.
		pub(crate) async fn next(&mut self) -> Result<RawEntry> {
			loop {
				if let Some(entry) = self.next_ready() {
					return Ok(entry);
				}

				// Seen before reading, so a commit after the read wakes us.
				self.wake.borrow_and_update();
				let entries = read(&self.db, self.version).await?;

				if entries.is_empty() {
					self.wake
						.changed()
						.await
						.map_err(|_| DbError::connection(Closed))?;
				} else {
					self.ready.extend(entries);
				}
			}
		}
	}

	impl std::fmt::Debug for Reader {
		fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
			f.debug_struct("Reader")
				.field("version", &self.version)
				.finish_non_exhaustive()
		}
	}

	/// Every handle to the database is gone, so no more changes will come.
	#[derive(Debug, thiserror::Error)]
	#[error("the database's live changes have stopped: every handle to it is closed")]
	struct Closed;

	/// A pack table's name in the database's own SQL: namespaced on
	/// Postgres, flat on SQLite.
	fn name(dialect: Dialect, table: &str) -> String {
		let table = leptoskit_db_schema::live::table(table);

		match dialect {
			Dialect::Postgres => {
				format!(
					"{}.{}",
					leptoskit_db_schema::quote_identifier(leptoskit_db_schema::live::NAMESPACE),
					leptoskit_db_schema::quote_identifier(&table.name)
				)
			}
			Dialect::Sqlite => leptoskit_db_schema::quote_identifier(&table.flat()),
		}
	}

	fn commits(dialect: Dialect) -> String {
		name(dialect, leptoskit_db_schema::live::COMMITS)
	}

	fn changes(dialect: Dialect) -> String {
		name(dialect, leptoskit_db_schema::live::CHANGES)
	}

	/// Whether the database has the live pack: a database without live
	/// tables has nothing to read.
	pub(crate) async fn has_log(db: &Db) -> Result<bool> {
		if db.live.has_log() {
			return Ok(true);
		}

		let sql = match db.dialect() {
			Dialect::Postgres => "SELECT to_regclass('live.commits') IS NOT NULL",
			Dialect::Sqlite => {
				"SELECT count(*) > 0 FROM sqlite_schema WHERE type = 'table' AND name = \
				 'live_commits'"
			}
		};

		let rows = db
			.fetch(Statement {
				sql: sql.to_string(),
				args: Vec::new(),
			})
			.await?;
		let found = matches!(
			rows.first().and_then(|row| row.first()),
			Some(Value::Bool(true) | Value::Int(1))
		);

		if found {
			db.live.found_log();
		}

		Ok(found)
	}

	/// The newest committed version, or 0 when nothing has committed or the
	/// database has no live tables.
	pub(crate) async fn latest_version(db: &Db) -> Result<i64> {
		if !has_log(db).await? {
			return Ok(0);
		}

		let rows = db
			.fetch(Statement {
				sql: format!(
					"SELECT coalesce(max(\"version\"), 0) FROM {}",
					commits(db.dialect())
				),
				args: Vec::new(),
			})
			.await?;

		match rows.first().and_then(|row| row.first()) {
			Some(Value::Int(version)) => Ok(*version),
			_ => Err(decode("the newest version isn't a number")),
		}
	}

	/// The commits after `after`, oldest first, with their changes, up to
	/// [`COMMITS_PER_READ`] at a time. One statement reads both, so a
	/// commit is never seen without the changes a concurrent prune removes
	/// with it.
	pub(crate) async fn read(db: &Db, after: i64) -> Result<Vec<RawEntry>> {
		if !has_log(db).await? {
			return Ok(Vec::new());
		}

		let rows = db.fetch(read_statement(db.dialect(), after)).await?;
		let mut entries: Vec<RawEntry> = Vec::new();

		for row in rows {
			let [
				version,
				at,
				actor,
				origin,
				lsn,
				fence,
				table,
				op,
				key,
				old,
				new,
				changed,
			]: [Value; 12] = row
				.try_into()
				.map_err(|_| decode("a row has the wrong columns"))?;
			let Value::Int(version) = version else {
				return Err(decode("a commit's version isn't a number"));
			};

			// Replication capture recorded where the commit's record starts;
			// one byte past it is past the whole record, since replay stops
			// only between records. Otherwise, the primary's insert position
			// as this statement ran, which no commit it sees can be past.
			let lsn = match (lsn, fence) {
				(Value::Int(lsn), _) => Some(lsn + 1),
				(_, Value::Int(fence)) => Some(fence),
				_ => None,
			};

			push_commit(&mut entries, version, at, actor, origin, lsn)?;
			push_change(&mut entries, table, op, key, old, new, changed)?;
		}

		Ok(entries)
	}

	/// The one statement that reads commits and their changes.
	fn read_statement(dialect: Dialect, after: i64) -> Statement {
		let (join, positions) = match dialect {
			Dialect::Postgres => {
				(
					"ch.\"txid\" = c.\"txid\"",
					"c.\"lsn\", (pg_current_wal_insert_lsn() - '0/0'::pg_lsn)::bigint",
				)
			}
			Dialect::Sqlite => ("ch.\"version\" = c.\"version\"", "NULL, NULL"),
		};

		Statement {
			sql: format!(
				"SELECT c.\"version\", c.\"committed_at\", c.\"actor\", c.\"origin\", \
				 {positions}, ch.\"table_name\", ch.\"op\", ch.\"key\", ch.\"old\", ch.\"new\", \
				 ch.\"changed\" FROM (SELECT * FROM {} WHERE \"version\" > {} ORDER BY \
				 \"version\" LIMIT {COMMITS_PER_READ}) c LEFT JOIN {} ch ON {join} ORDER BY \
				 c.\"version\", ch.\"seq\"",
				commits(dialect),
				placeholder(dialect, 1),
				changes(dialect)
			),
			args: vec![Value::Int(after)],
		}
	}

	/// Starts a commit the row belongs to, unless it is the one before.
	fn push_commit(
		entries: &mut Vec<RawEntry>,
		version: i64,
		at: Value,
		actor: Value,
		origin: Value,
		lsn: Option<i64>,
	) -> Result<()> {
		let same = match entries.last() {
			Some(RawEntry::Commit(commit)) => commit.version == version,
			Some(RawEntry::Pruned { through }) => *through == version,
			None => false,
		};

		if same {
			return Ok(());
		}

		let origin = text(origin)?;
		entries.push(if origin == PRUNED {
			RawEntry::Pruned { through: version }
		} else {
			RawEntry::Commit(RawCommit {
				version,
				committed_at: timestamp(at)?,
				actor: uuid(actor)?,
				origin,
				changes: Vec::new(),
				lsn,
			})
		});

		Ok(())
	}

	/// Adds the row's change to the commit it belongs to, if it has one.
	fn push_change(
		entries: &mut [RawEntry],
		table: Value,
		op: Value,
		key: Value,
		old: Value,
		new: Value,
		changed: Value,
	) -> Result<()> {
		// A commit with no changes left, or a pruned marker.
		let (Some(RawEntry::Commit(commit)), Value::Text(table)) = (entries.last_mut(), table)
		else {
			return Ok(());
		};

		let op = match text(op)?.as_str() {
			"I" => ChangeOp::Insert,
			"U" => ChangeOp::Update,
			"D" => ChangeOp::Delete,
			other => return Err(decode(&format!("`{other}` isn't a change's operation"))),
		};

		let Some(serde_json::Value::Array(key)) = json(key)? else {
			return Err(decode("a change's key isn't an array"));
		};

		let changed = match json(changed)? {
			Some(serde_json::Value::Array(names)) => {
				names
					.into_iter()
					.filter_map(|name| {
						match name {
							serde_json::Value::String(name) => Some(name),
							_ => None,
						}
					})
					.collect()
			}
			_ => Vec::new(),
		};

		commit.changes.push(RawChange {
			table: table_name(&table),
			op,
			key,
			old: object(old)?,
			new: object(new)?,
			changed,
		});
		Ok(())
	}

	/// `$n` or `?n`.
	fn placeholder(dialect: Dialect, number: usize) -> String {
		match dialect {
			Dialect::Postgres => format!("${number}"),
			Dialect::Sqlite => format!("?{number}"),
		}
	}

	/// A table's name as the capture trigger wrote it: `posts` or
	/// `billing.invoices`.
	fn table_name(name: &str) -> TableName {
		match name.split_once('.') {
			Some((namespace, name)) => TableName::namespaced(namespace, name),
			None => TableName::new(name),
		}
	}

	fn decode(message: &str) -> DbError {
		DbError::Decode(DecodeError::custom(format!("the change log: {message}")))
	}

	fn text(value: Value) -> Result<String> {
		match value {
			Value::Text(text) => Ok(text),
			other => Err(decode(&format!("expected text, found {}", other.kind()))),
		}
	}

	fn timestamp(value: Value) -> Result<Timestamp> {
		match value {
			Value::Timestamp(at) => Ok(at),
			Value::Text(text) => parse_timestamp(&text).ok_or_else(|| decode("a bad timestamp")),
			other => {
				Err(decode(&format!(
					"expected a timestamp, found {}",
					other.kind()
				)))
			}
		}
	}

	fn uuid(value: Value) -> Result<Option<Uuid>> {
		match value {
			Value::Null => Ok(None),
			Value::Uuid(uuid) => Ok(Some(uuid)),
			Value::Bytes(bytes) => {
				Uuid::from_slice(&bytes)
					.map(Some)
					.map_err(|_| decode("an actor isn't a UUID"))
			}
			other => Err(decode(&format!("expected a UUID, found {}", other.kind()))),
		}
	}

	/// JSON as Postgres returns it, or as SQLite's text.
	fn json(value: Value) -> Result<Option<serde_json::Value>> {
		match value {
			Value::Null => Ok(None),
			Value::Json(json) => Ok(Some(json)),
			Value::Text(text) => {
				serde_json::from_str(&text)
					.map(Some)
					.map_err(|error| decode(&error.to_string()))
			}
			other => Err(decode(&format!("expected JSON, found {}", other.kind()))),
		}
	}

	fn object(value: Value) -> Result<Option<serde_json::Map<String, serde_json::Value>>> {
		match json(value)? {
			None | Some(serde_json::Value::Null) => Ok(None),
			Some(serde_json::Value::Object(map)) => Ok(Some(map)),
			Some(_) => Err(decode("a change's row isn't an object")),
		}
	}

	/// The commits older than `retention`, and their changes, deleted in
	/// batches of whole commits. The newest pruned commit stays first, as a
	/// marker: a reader behind it learns it missed changes, and SQLite's
	/// next version still follows it. The batches then go down from the
	/// marker, so whatever a reader behind it can still read is whole, and
	/// the next thing it reads past that is the marker. Returns how many
	/// commits were removed.
	///
	/// On Postgres one server prunes at a time; another finding it busy
	/// stops.
	pub(crate) async fn prune(db: &Db, retention: std::time::Duration) -> Result<u64> {
		if !has_log(db).await? {
			return Ok(0);
		}

		let dialect = db.dialect();
		let cutoff = crate::now()
			- chrono::Duration::from_std(retention).unwrap_or_else(|_| chrono::Duration::zero());
		let rows = db
			.fetch(Statement {
				sql: format!(
					"SELECT max(\"version\") FROM {} WHERE \"committed_at\" <= {}",
					commits(dialect),
					placeholder(dialect, 1)
				),
				args: vec![Value::Timestamp(cutoff)],
			})
			.await?;
		let Some(Value::Int(through)) = rows
			.into_iter()
			.next()
			.and_then(|row| row.into_iter().next())
		else {
			return Ok(0);
		};

		db.execute(Statement {
			sql: format!(
				"UPDATE {} SET \"origin\" = {}, \"actor\" = NULL WHERE \"version\" = {}",
				commits(dialect),
				placeholder(dialect, 1),
				placeholder(dialect, 2)
			),
			args: vec![Value::Text(PRUNED.to_string()), Value::Int(through)],
		})
		.await?;

		let mut removed = 0;

		loop {
			let batch = db
				.transaction(async |tx| prune_batch(tx, dialect, through).await)
				.await?;

			match batch {
				Some(count) => removed += count,
				None => break,
			}
		}

		Ok(removed)
	}

	/// The commits a batch removes at most.
	const PRUNE_BATCH: usize = 1000;

	/// Removes the newest commits below the marker at `through`, and their
	/// changes, or the marker's own changes once none are left below it.
	/// `None` when there's nothing left to do, or another server is pruning.
	async fn prune_batch(tx: &crate::Tx, dialect: Dialect, through: i64) -> Result<Option<u64>> {
		if dialect == Dialect::Postgres {
			let rows = tx
				.fetch(Statement {
					sql: format!(
						"SELECT pg_try_advisory_xact_lock({})",
						leptoskit_db_schema::live::PRUNE_LOCK
					),
					args: Vec::new(),
				})
				.await?;

			if !matches!(
				rows.first().and_then(|row| row.first()),
				Some(Value::Bool(true))
			) {
				return Ok(None);
			}
		}

		let rows = tx
			.fetch(Statement {
				sql: format!(
					"SELECT \"version\" FROM {} WHERE \"version\" < {} ORDER BY \"version\" DESC \
					 LIMIT {PRUNE_BATCH}",
					commits(dialect),
					placeholder(dialect, 1)
				),
				args: vec![Value::Int(through)],
			})
			.await?;
		// The batch's commits are `from..to`; the marker's own, `through`.
		let (from, to, done) = match rows.last().and_then(|row| row.first()) {
			Some(Value::Int(lowest)) => (*lowest, through - 1, false),
			_ => (through, through, true),
		};

		let delete_changes = match dialect {
			Dialect::Postgres => {
				format!(
					"DELETE FROM {} WHERE \"txid\" IN (SELECT \"txid\" FROM {} WHERE \"version\" \
					 >= $1 AND \"version\" <= $2)",
					changes(dialect),
					commits(dialect)
				)
			}
			Dialect::Sqlite => {
				format!(
					"DELETE FROM {} WHERE \"version\" >= ?1 AND \"version\" <= ?2",
					changes(dialect)
				)
			}
		};

		tx.execute(Statement {
			sql: delete_changes,
			args: vec![Value::Int(from), Value::Int(to)],
		})
		.await?;

		// The registrations `live.seal()` leaves behind, rather than reading
		// the table to delete its own.
		if dialect == Dialect::Postgres {
			tx.execute(Statement {
				sql: format!(
					"DELETE FROM {} WHERE \"txid\" IN (SELECT \"txid\" FROM {} WHERE \"version\" \
					 >= $1 AND \"version\" <= $2)",
					name(dialect, leptoskit_db_schema::live::PENDING),
					commits(dialect)
				),
				args: vec![Value::Int(from), Value::Int(to)],
			})
			.await?;
		}

		if done {
			return Ok(None);
		}

		let removed = tx
			.execute(Statement {
				sql: format!(
					"DELETE FROM {} WHERE \"version\" >= {} AND \"version\" <= {}",
					commits(dialect),
					placeholder(dialect, 1),
					placeholder(dialect, 2)
				),
				args: vec![Value::Int(from), Value::Int(to)],
			})
			.await?;

		Ok(Some(removed))
	}
}

#[cfg(feature = "postgres")]
pub(crate) use reading::PRUNED;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub(crate) use reading::Reader;
#[cfg(feature = "postgres")]
pub(crate) use reading::has_log;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub(crate) use reading::latest_version;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub(crate) use reading::prune;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub(crate) use reading::read;

/// RFC 3339, as Postgres and SQLite both write timestamps into JSON, with or
/// without an offset (read as UTC).
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub(crate) fn parse_timestamp(text: &str) -> Option<Timestamp> {
	chrono::DateTime::parse_from_rfc3339(text)
		.map(|at| at.to_utc())
		.ok()
		.or_else(|| {
			chrono::NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M:%S%.f")
				.ok()
				.map(|at| at.and_utc())
		})
}

/// On SQLite, just before `COMMIT`: stamps the next version on every change
/// without one, whichever process wrote it, and records the commit. Returns
/// the version, or `None` when there was nothing to stamp or the database
/// has no live tables.
#[cfg(feature = "sqlite")]
pub(crate) fn seal_sqlite(
	connection: &rusqlite::Connection,
	actor: Option<Uuid>,
	origin: Option<&str>,
) -> rusqlite::Result<Option<i64>> {
	use rusqlite::OptionalExtension;

	let has_log = connection
		.prepare_cached(
			"SELECT 1 FROM sqlite_schema WHERE type = 'table' AND name = 'live_changes'",
		)?
		.query_row([], |_| Ok(()))
		.optional()?
		.is_some();

	if !has_log {
		return Ok(None);
	}

	let version: i64 = connection
		.prepare_cached("SELECT coalesce(max(\"version\"), 0) + 1 FROM \"live_commits\"")?
		.query_row([], |row| row.get(0))?;
	let stamped = connection
		.prepare_cached("UPDATE \"live_changes\" SET \"version\" = ?1 WHERE \"version\" IS NULL")?
		.execute([version])?;

	if stamped == 0 {
		return Ok(None);
	}

	let committed_at = crate::now()
		.format(crate::sqlite::TIMESTAMP_FORMAT)
		.to_string();
	connection
		.prepare_cached(
			"INSERT INTO \"live_commits\" (\"version\", \"txid\", \"committed_at\", \"actor\", \
			 \"origin\") VALUES (?1, NULL, ?2, ?3, ?4)",
		)?
		.execute(rusqlite::params![
			version,
			committed_at,
			actor.map(|actor| actor.as_bytes().to_vec()),
			origin.unwrap_or(leptoskit_db_schema::live::DEFAULT_ORIGIN),
		])?;
	Ok(Some(version))
}
