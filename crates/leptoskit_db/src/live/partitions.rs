//! The change log's partitions, Postgres only: making them ahead of the
//! rows being written, and pruning by dropping whole ones.
//!
//! `live.commits` is partitioned by version and `live.changes` by `seq`,
//! the order changes were written, in ranges, plus a default partition that
//! holds whatever no range covers: everything written before maintenance
//! first ran, and anything written while it falls behind. Ranges are made
//! above every row the default partition holds, so making one never moves
//! rows.
//!
//! Pruning keeps the newest commit older than the retention as a marker,
//! in place, so a reader behind it hears that it may have missed changes,
//! then drops every commits range below the marker and deletes the default
//! partition's commits below it. A changes range goes once none of its
//! changes belongs to a commit still in the log, checked under the lock that
//! dropping takes anyway, so a transaction still writing to it is waited
//! for, not lost. Readers see whole commits throughout: whatever they can
//! still read below the marker is untouched, and the marker comes after it.
//!
//! Each step is its own short transaction, under the pruning advisory lock,
//! so servers sharing the database take turns. Dropping a partition locks
//! its table for a moment; a step that can't get the lock within a second
//! gives up until the next round, rather than holding up writers.

use std::time::Duration;

use leptoskit_db_schema::live;

use crate::Connection;
use crate::Db;
use crate::Read;
use crate::Result;
use crate::Value;
use crate::sql::Statement;

/// The smallest range made, in versions or changes.
const MIN_CHUNK: i64 = 4096;

/// The largest range made.
const MAX_CHUNK: i64 = 1 << 24;

/// How long a step waits for the locks dropping or making a partition
/// takes, before trying again next round.
const LOCK_TIMEOUT: &str = "1s";

/// One partitioned table of the log.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Log {
	Commits,
	Changes,
}

impl Log {
	fn table(self) -> &'static str {
		match self {
			Self::Commits => live::COMMITS,
			Self::Changes => live::CHANGES,
		}
	}

	fn column(self) -> &'static str {
		match self {
			Self::Commits => "version",
			Self::Changes => "seq",
		}
	}

	fn sequence(self) -> &'static str {
		match self {
			Self::Commits => "version_seq",
			Self::Changes => "change_seq",
		}
	}

	fn qualified(self) -> String {
		format!("live.{}", self.table())
	}

	fn default_partition(self) -> String {
		format!("live.{}_default", self.table())
	}
}

/// How fast a table's key grew between maintenance rounds, which sizes the
/// next ranges so each holds a fraction of the retention.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Growth {
	last: Option<(std::time::Instant, i64)>,
}

/// One range partition: its table's name and the keys it covers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Range {
	pub(crate) name: String,
	pub(crate) lower: i64,
	pub(crate) upper: i64,
}

/// Whether the log is partitioned: a database migrated before partitioning
/// prunes by deleting rows instead.
pub(crate) async fn partitioned(db: &Db) -> Result<bool> {
	let rows = db
		.fetch(Statement {
			sql: "SELECT EXISTS (SELECT FROM pg_catalog.pg_partitioned_table WHERE partrelid = \
			      to_regclass('live.commits'))"
				.to_string(),
			args: Vec::new(),
		})
		.await?;
	Ok(matches!(
		rows.first().and_then(|row| row.first()),
		Some(Value::Bool(true))
	))
}

/// The range partitions of `log`, lowest first.
pub(crate) async fn ranges(db: &impl Read, log: Log) -> Result<Vec<Range>> {
	let rows = db
		.fetch(Statement {
			sql: "SELECT c.relname, pg_get_expr(c.relpartbound, c.oid) FROM \
			      pg_catalog.pg_inherits i JOIN pg_catalog.pg_class c ON c.oid = i.inhrelid WHERE \
			      i.inhparent = to_regclass($1)"
				.to_string(),
			args: vec![Value::Text(log.qualified())],
		})
		.await?;
	let mut ranges: Vec<Range> = rows
		.into_iter()
		.filter_map(|row| {
			let mut row = row.into_iter();
			let (Some(Value::Text(name)), Some(Value::Text(bound))) = (row.next(), row.next())
			else {
				return None;
			};
			let (lower, upper) = bounds(&bound)?;
			Some(Range { name, lower, upper })
		})
		.collect();
	ranges.sort_by_key(|range| range.lower);
	Ok(ranges)
}

/// A range's bounds from Postgres's description of it, `FOR VALUES FROM
/// ('1') TO ('4097')`; `None` for the default partition.
fn bounds(bound: &str) -> Option<(i64, i64)> {
	let number = |text: &str| -> Option<i64> {
		text.trim()
			.trim_start_matches('(')
			.trim_end_matches(')')
			.trim_matches('\'')
			.parse()
			.ok()
	};
	let rest = bound.strip_prefix("FOR VALUES FROM ")?;
	let (lower, upper) = rest.split_once(" TO ")?;
	Some((number(lower)?, number(upper)?))
}

/// One integer from a one-row, one-column query, or `None` for `NULL`.
async fn scalar(db: &impl Read, sql: String, args: Vec<Value>) -> Result<Option<i64>> {
	let rows = db.fetch(Statement { sql, args }).await?;
	Ok(match rows.first().and_then(|row| row.first()) {
		Some(Value::Int(value)) => Some(*value),
		_ => None,
	})
}

/// Whether this transaction got the pruning lock: another server holding it
/// is maintaining the log already.
async fn take_turn(tx: &crate::Tx) -> Result<bool> {
	let rows = tx
		.fetch(Statement {
			sql: format!(
				"SELECT pg_try_advisory_xact_lock({}), set_config('lock_timeout', \
				 '{LOCK_TIMEOUT}', true)",
				live::PRUNE_LOCK
			),
			args: Vec::new(),
		})
		.await?;
	Ok(matches!(
		rows.first().and_then(|row| row.first()),
		Some(Value::Bool(true))
	))
}

/// Makes ranges of `log` ahead of its key, so writes land in a range rather
/// than the default partition: enough to cover twice what it grew by since
/// the last round, and at least two ranges' worth. Each range holds about a
/// quarter of what `retention` takes to write, within bounds. Returns how
/// many it made.
pub(crate) async fn make_ahead(
	db: &Db,
	log: Log,
	retention: Duration,
	growth: &mut Growth,
) -> Result<usize> {
	let current = scalar(
		db,
		format!("SELECT last_value FROM live.{}", log.sequence()),
		Vec::new(),
	)
	.await?
	.unwrap_or(1);
	let grew = match growth.last {
		Some((at, before)) => {
			let elapsed = at.elapsed().as_secs_f64().max(1.0);
			let per_second = (current - before).max(0) as f64 / elapsed;
			Some((per_second * elapsed, per_second))
		}
		None => None,
	};
	growth.last = Some((std::time::Instant::now(), current));

	let chunk = grew.map_or(MIN_CHUNK, |(_, per_second)| {
		let wanted = (per_second * retention.as_secs_f64() / 4.0) as i64;
		(wanted.clamp(MIN_CHUNK, MAX_CHUNK) as u64).next_power_of_two() as i64
	});
	let headroom = (2 * chunk).max(2 * grew.map_or(0, |(grew, _)| grew as i64));

	db.transaction(async |tx| -> Result<usize> {
		if !take_turn(tx).await? {
			return Ok(0);
		}

		let default_max = scalar(
			tx,
			format!(
				"SELECT max({}) FROM {}",
				log.column(),
				log.default_partition()
			),
			Vec::new(),
		)
		.await?;
		let ranges = ranges(tx, log).await?;
		let mut start = ranges
			.last()
			.map_or(current, |range| range.upper)
			.max(default_max.map_or(i64::MIN, |max| max + 1));
		let mut made = 0;

		while start < current + headroom {
			let end = start.saturating_add(chunk);
			tx.execute(Statement {
				sql: format!(
					"CREATE TABLE live.{} PARTITION OF {} FOR VALUES FROM ({start}) TO ({end})",
					leptoskit_db_schema::quote_identifier(&format!("{}_p{start}", log.table())),
					log.qualified()
				),
				args: Vec::new(),
			})
			.await?;
			start = end;
			made += 1;
		}

		Ok(made)
	})
	.await
}

/// Prunes the partitioned log: keeps the newest commit older than
/// `retention` as a marker, then drops and deletes what's below it, as the
/// module explains. Returns how many commits it removed.
pub(crate) async fn prune(db: &Db, retention: Duration) -> Result<u64> {
	// Not `crate::now()`, which rounds down to the millisecond and would
	// leave out a commit made in the same one.
	let cutoff = chrono::Utc::now()
		- chrono::Duration::from_std(retention).unwrap_or_else(|_| chrono::Duration::zero());
	let Some(through) = mark(db, cutoff).await? else {
		return Ok(0);
	};
	let mut removed = 0;

	for range in ranges(db, Log::Commits).await? {
		if range.upper > through {
			break;
		}

		match drop_commits(db, &range, through).await? {
			Some(count) => removed += count,
			None => return Ok(removed),
		}
	}

	removed += delete_old_commits(db, through).await?;

	for range in ranges(db, Log::Changes).await? {
		if !drop_orphaned_changes(db, &range).await? {
			break;
		}
	}

	delete_orphans(db).await?;
	Ok(removed)
}

/// Makes the newest commit at or before `cutoff` the marker, in place, and
/// returns its version: `None` when there's none, or another server is
/// pruning.
///
/// Versions are allocated in commit order, so the commit is found by
/// searching versions rather than reading every commit's time.
async fn mark(db: &Db, cutoff: crate::Timestamp) -> Result<Option<i64>> {
	db.transaction(async |tx| -> Result<Option<i64>> {
		if !take_turn(tx).await? {
			return Ok(None);
		}

		let Some(through) = newest_before(tx, cutoff).await? else {
			return Ok(None);
		};

		// The marker keeps no changes of its own.
		tx.execute(Statement {
			sql: "DELETE FROM live.changes WHERE txid = (SELECT txid FROM live.commits WHERE \
			      version = $1)"
				.to_string(),
			args: vec![Value::Int(through)],
		})
		.await?;
		tx.execute(Statement {
			sql: "UPDATE live.commits SET origin = $1, actor = NULL, txid = NULL WHERE version = \
			      $2"
			.to_string(),
			args: vec![
				Value::Text(super::log::PRUNED.to_string()),
				Value::Int(through),
			],
		})
		.await?;
		Ok(Some(through))
	})
	.await
}

/// The newest commit at or before `cutoff`, by a binary search over
/// versions, each step one look-up by key.
async fn newest_before(tx: &crate::Tx, cutoff: crate::Timestamp) -> Result<Option<i64>> {
	let at_or_after = async |version: i64| -> Result<Option<(i64, crate::Timestamp)>> {
		let rows = tx
			.fetch(Statement {
				sql: "SELECT version, committed_at FROM live.commits WHERE version >= $1 ORDER BY \
				      version LIMIT 1"
					.to_string(),
				args: vec![Value::Int(version)],
			})
			.await?;
		Ok(rows.into_iter().next().and_then(|row| {
			match (row.first(), row.get(1)) {
				(Some(Value::Int(version)), Some(Value::Timestamp(at))) => Some((*version, *at)),
				_ => None,
			}
		}))
	};

	let Some((oldest, at)) = at_or_after(i64::MIN).await? else {
		return Ok(None);
	};

	if at > cutoff {
		return Ok(None);
	}

	// `low` is always a commit at or before the cutoff; `high` bounds the
	// search from above.
	let mut low = oldest;
	let mut high = scalar(
		tx,
		"SELECT max(version) FROM live.commits".to_string(),
		Vec::new(),
	)
	.await?
	.unwrap_or(oldest);

	while low < high {
		let middle = low + (high - low + 1) / 2;

		match at_or_after(middle).await? {
			Some((version, at)) if at <= cutoff && version <= high => low = version,
			_ => high = middle - 1,
		}
	}

	Ok(Some(low))
}

/// Drops a commits range wholly below the marker at `through`, if the
/// marker is still there. Returns how many commits it held, or `None` when
/// this server should stop: another is pruning, or the lock wasn't free.
async fn drop_commits(db: &Db, range: &Range, through: i64) -> Result<Option<u64>> {
	let outcome = db
		.transaction(async |tx| -> Result<Option<u64>> {
			if !take_turn(tx).await? {
				return Ok(None);
			}

			let marked = scalar(
				tx,
				"SELECT count(*) FROM live.commits WHERE version = $1 AND origin = $2".to_string(),
				vec![
					Value::Int(through),
					Value::Text(super::log::PRUNED.to_string()),
				],
			)
			.await?;

			if marked != Some(1) {
				return Ok(None);
			}

			let partition = format!(
				"live.{}",
				leptoskit_db_schema::quote_identifier(&range.name)
			);
			let count = scalar(tx, format!("SELECT count(*) FROM {partition}"), Vec::new()).await?;
			tx.execute(Statement {
				sql: format!("DROP TABLE {partition}"),
				args: Vec::new(),
			})
			.await?;
			Ok(Some(count.unwrap_or(0) as u64))
		})
		.await;

	gave_up_on_lock(outcome)
}

/// Deletes the default partition's commits below the marker.
async fn delete_old_commits(db: &Db, through: i64) -> Result<u64> {
	db.transaction(async |tx| -> Result<u64> {
		if !take_turn(tx).await? {
			return Ok(0);
		}

		tx.execute(Statement {
			sql: format!(
				"DELETE FROM {} WHERE version < $1",
				Log::Commits.default_partition()
			),
			args: vec![Value::Int(through)],
		})
		.await
	})
	.await
}

/// Drops a changes range once none of its changes belongs to a commit
/// still in the log. The table's lock is taken first, so a transaction
/// still writing to the range finishes before the check sees its rows.
/// Whether it dropped it.
async fn drop_orphaned_changes(db: &Db, range: &Range) -> Result<bool> {
	let outcome = db
		.transaction(async |tx| -> Result<Option<u64>> {
			if !take_turn(tx).await? {
				return Ok(None);
			}

			tx.execute(Statement {
				sql: "LOCK TABLE live.changes IN ACCESS EXCLUSIVE MODE".to_string(),
				args: Vec::new(),
			})
			.await?;
			let partition = format!(
				"live.{}",
				leptoskit_db_schema::quote_identifier(&range.name)
			);
			let kept = scalar(
				tx,
				format!(
					"SELECT count(*) FROM (SELECT FROM live.commits c WHERE c.txid IS NOT NULL \
					 AND EXISTS (SELECT FROM {partition} ch WHERE ch.txid = c.txid) LIMIT 1) kept"
				),
				Vec::new(),
			)
			.await?;

			if kept != Some(0) {
				return Ok(None);
			}

			tx.execute(Statement {
				sql: format!("DROP TABLE {partition}"),
				args: Vec::new(),
			})
			.await?;
			Ok(Some(0))
		})
		.await;

	Ok(gave_up_on_lock(outcome)?.is_some())
}

/// Deletes the default partition's changes, and the registrations triggers
/// leave in `live.pending`, whose commits are no longer in the log.
async fn delete_orphans(db: &Db) -> Result<()> {
	db.transaction(async |tx| -> Result<()> {
		if !take_turn(tx).await? {
			return Ok(());
		}

		for sql in [
			format!(
				"DELETE FROM {} ch WHERE NOT EXISTS (SELECT FROM live.commits c WHERE c.txid = \
				 ch.txid)",
				Log::Changes.default_partition()
			),
			"DELETE FROM live.pending p WHERE NOT EXISTS (SELECT FROM live.commits c WHERE c.txid \
			 = p.txid)"
				.to_string(),
		] {
			tx.execute(Statement {
				sql,
				args: Vec::new(),
			})
			.await?;
		}

		Ok(())
	})
	.await
}

/// A step that timed out waiting for a lock gives up until the next round.
fn gave_up_on_lock<T>(outcome: Result<Option<T>>) -> Result<Option<T>> {
	match outcome {
		Err(error) if error.to_string().contains("lock timeout") => Ok(None),
		outcome => outcome,
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn bounds_read_as_postgres_writes_them() {
		assert_eq!(bounds("FOR VALUES FROM ('1') TO ('4097')"), Some((1, 4097)));
		assert_eq!(bounds("FOR VALUES FROM (-5) TO (10)"), Some((-5, 10)));
		assert_eq!(bounds("DEFAULT"), None);
	}
}
