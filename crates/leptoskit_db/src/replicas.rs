//! Read replicas: Postgres standbys that take reads off the primary.
//!
//! ```ignore
//! let db = Db::connect(&primary_url).await?.with_replica(&replica_url)?;
//!
//! // Reads stay on the primary unless asked:
//! let report = Report::query().all(&db.on_replica()).await?;
//!
//! // Or only from a replica that has replayed a given commit:
//! if let Some(replica) = db.on_replica_at(commit.lsn.unwrap(), Duration::from_millis(50)).await {
//! 	let rows = Message::query().all(&replica).await?;
//! }
//! ```
//!
//! A standby replays the primary's write-ahead log a moment behind it, so
//! a read from one may not see a write that just committed. Ordinary reads
//! therefore stay on the primary: [`Db::on_replica`] opts in where a stale
//! read is fine, and [`Db::on_replica_at`] picks a replica only once it has
//! replayed past a position, such as a change log commit's
//! [`lsn`](crate::live::Commit::lsn), which is how live queries re-run on
//! replicas without ever showing an older state than the commit that
//! caused them. A handle on a replica sends it only `SELECT`s: writes, and
//! transactions, still go to the primary.

use std::sync::Arc;
use std::sync::atomic::AtomicI64;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::time::Duration;

use crate::Result;
use crate::Value;
use crate::sql::Statement;

/// How often [`Db::on_replica_at`] asks the replicas again while it waits.
const POLL: Duration = Duration::from_millis(5);

/// The read replicas a database handle may read from.
#[derive(Default)]
pub(crate) struct Replicas {
	pub(crate) list: Vec<Arc<Replica>>,
	next: AtomicUsize,
}

impl Replicas {
	/// These replicas and `replica`.
	#[cfg(feature = "postgres")]
	pub(crate) fn with(&self, replica: Replica) -> Self {
		let mut list = self.list.clone();
		list.push(Arc::new(replica));
		Self {
			list,
			next: AtomicUsize::new(0),
		}
	}

	/// The next replica, taking turns.
	pub(crate) fn any(&self) -> Option<Arc<Replica>> {
		if self.list.is_empty() {
			return None;
		}

		let index = self.next.fetch_add(1, Ordering::Relaxed) % self.list.len();
		self.list.get(index).cloned()
	}

	/// A replica that has replayed to `lsn`, asking each again every
	/// [`POLL`] until `wait` has passed.
	pub(crate) async fn at(&self, lsn: i64, wait: Duration) -> Option<Arc<Replica>> {
		let deadline = tokio::time::Instant::now() + wait;

		loop {
			// The cached position only moves forward, as replay does, so a
			// replica known to be past `lsn` needs no question.
			if let Some(replica) = self
				.list
				.iter()
				.find(|replica| replica.replayed.load(Ordering::Relaxed) >= lsn)
			{
				return Some(Arc::clone(replica));
			}

			for replica in &self.list {
				// Opening a connection or asking a stalled standby must use
				// the same budget as waiting for replay to catch up.
				let replayed = tokio::time::timeout_at(deadline, replica.refresh())
					.await
					.ok()?;

				if replayed >= lsn {
					return Some(Arc::clone(replica));
				}
			}

			let now = tokio::time::Instant::now();

			if now >= deadline {
				return None;
			}

			tokio::time::sleep(POLL.min(deadline - now)).await;
		}
	}
}

/// One read replica: its pool, and how far it has replayed.
pub(crate) struct Replica {
	#[cfg(feature = "postgres")]
	pool: crate::postgres::Pool,
	/// Where it is, without credentials, for messages.
	name: String,
	/// The newest replay position seen, as bytes into the write-ahead log;
	/// `i64::MIN` until one is, or when the server isn't a standby.
	replayed: AtomicI64,
	/// Reads it has served.
	reads: AtomicU64,
}

impl Replica {
	/// A replica at the Postgres `url`; its connections open on first use.
	#[cfg(feature = "postgres")]
	pub(crate) fn connect(url: &str, options: crate::PoolOptions) -> Result<Self> {
		Ok(Self {
			pool: crate::postgres::Pool::connect(url, options)?,
			name: redact(url),
			replayed: AtomicI64::new(i64::MIN),
			reads: AtomicU64::new(0),
		})
	}

	/// Asks the replica how far it has replayed, and remembers it.
	#[cfg_attr(
		not(feature = "postgres"),
		expect(clippy::unused_async, reason = "only Postgres has replicas")
	)]
	async fn refresh(&self) -> i64 {
		#[cfg(feature = "postgres")]
		{
			let replayed = self
				.pool
				.fetch(Statement {
					sql: "SELECT (pg_last_wal_replay_lsn() - '0/0'::pg_lsn)::bigint".into(),
					args: Vec::new(),
				})
				.await;

			// Not a standby (no replay position), or unreachable: never
			// caught up.
			if let Ok(rows) = replayed
				&& let Some(Value::Int(position)) = rows.first().and_then(|row| row.first())
			{
				return self
					.replayed
					.fetch_max(*position, Ordering::Relaxed)
					.max(*position);
			}
		}

		self.replayed.load(Ordering::Relaxed)
	}

	/// Reads on the replica, as the primary would with `settings`.
	#[cfg_attr(
		not(feature = "postgres"),
		expect(
			unused_variables,
			clippy::unused_async,
			reason = "only Postgres has replicas"
		)
	)]
	pub(crate) async fn fetch(
		&self,
		settings: Option<Vec<(String, String)>>,
		statement: Statement,
	) -> Result<Vec<Vec<Value>>> {
		self.reads.fetch_add(1, Ordering::Relaxed);

		#[cfg(feature = "postgres")]
		return match settings {
			Some(settings) => self.pool.fetch_secured(&settings, statement).await,
			None => self.pool.fetch(statement).await,
		};

		#[cfg(not(feature = "postgres"))]
		unreachable!("only Postgres has replicas")
	}

	pub(crate) fn stats(&self) -> ReplicaStats {
		let replayed = self.replayed.load(Ordering::Relaxed);
		ReplicaStats {
			name: self.name.clone(),
			replayed: (replayed != i64::MIN).then_some(replayed),
			reads: self.reads.load(Ordering::Relaxed),
		}
	}
}

/// Whether `sql` is a plain `SELECT`, which a replica can run: anything
/// else, such as an `INSERT … RETURNING` or a `WITH` that may write, runs on
/// the primary.
pub(crate) fn reads_only(sql: &str) -> bool {
	sql.trim_start()
		.get(..6)
		.is_some_and(|start| start.eq_ignore_ascii_case("select"))
}

/// What one read replica has done, from [`Db::replica_stats`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplicaStats {
	/// Where it is: host, port and database, without credentials.
	pub name: String,
	/// The newest write-ahead log position this process has seen it
	/// replay, as bytes from the start of the log; `None` before it has
	/// answered, or when it isn't a standby.
	pub replayed: Option<i64>,
	/// Statements it has run for this process.
	pub reads: u64,
}

/// `url` without its user and password.
#[cfg(feature = "postgres")]
fn redact(url: &str) -> String {
	let Some((scheme, rest)) = url.split_once("://") else {
		return "a replica".to_owned();
	};
	let rest = rest.rsplit_once('@').map_or(rest, |(_, host)| host);
	let rest = rest.split_once('?').map_or(rest, |(host, _)| host);
	format!("{scheme}://{rest}")
}

/// Asked for a replica the database can't have.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub(crate) struct NoReplica(pub(crate) &'static str);

impl crate::Db {
	/// The same database, also reading from the Postgres standby at `url`.
	/// Call it once per replica, right after connecting; its connections
	/// open on first use.
	///
	/// Nothing reads from it until asked: see [`Db::on_replica`] and
	/// [`Db::on_replica_at`]. Live queries re-run on replicas that have
	/// caught up with the commit that caused the re-run.
	///
	/// # Errors
	///
	/// When this database is SQLite, which has no replicas, or `url` isn't
	/// a Postgres URL.
	pub fn with_replica(&self, url: &str) -> Result<Self> {
		#[cfg(feature = "postgres")]
		if self.postgres_url().is_some()
			&& (url.starts_with("postgres://") || url.starts_with("postgresql://"))
		{
			let replica = Replica::connect(url, self.pool_options())?;
			let mut db = self.clone();
			db.replicas = Arc::new(self.replicas.with(replica));
			return Ok(db);
		}

		let _ = url;
		Err(crate::DbError::connection(NoReplica(
			"read replicas are Postgres standbys of a Postgres primary: give `with_replica` a \
			 `postgres://` URL, on a database connected to Postgres",
		)))
	}

	/// [`Db::with_replica`] for each environment variable in `names`, such
	/// as `[db] replica_url_envs`. Variables that aren't set, or are empty,
	/// are skipped, so development runs without replicas.
	///
	/// # Errors
	///
	/// As [`Db::with_replica`], for a variable that is set.
	pub fn with_replicas_from_env<S: AsRef<str>>(&self, names: &[S]) -> Result<Self> {
		let mut db = self.clone();

		for name in names {
			if let Ok(url) = std::env::var(name.as_ref())
				&& !url.trim().is_empty()
			{
				db = db.with_replica(url.trim())?;
			}
		}

		Ok(db)
	}

	/// How many read replicas this database has.
	pub fn replica_count(&self) -> usize {
		self.replicas.list.len()
	}

	/// What each read replica has done, in the order they were added.
	pub fn replica_stats(&self) -> Vec<ReplicaStats> {
		self.replicas
			.list
			.iter()
			.map(|replica| replica.stats())
			.collect()
	}

	/// The same handle, reading from a replica, taking turns between them,
	/// or from the primary when there are none. Only its `SELECT`s go to
	/// the replica: writes, and transactions, still go to the primary.
	///
	/// A replica replays the primary's writes a moment late, so a read
	/// through this may not see a write that just committed, even this
	/// request's own. Use it for reads that can be a little stale, such as
	/// reports and search pages.
	#[must_use]
	pub fn on_replica(&self) -> Self {
		let mut db = self.clone();
		db.replica = self.replicas.any();
		db
	}

	/// The same handle, reading from the primary again.
	#[must_use]
	pub fn on_primary(&self) -> Self {
		let mut db = self.clone();
		db.replica = None;
		db
	}

	/// The same handle, reading from wherever `other` reads: the same
	/// replica, or the primary. For moving a request's handle onto a
	/// replica that [`Db::on_replica_at`] found.
	#[must_use]
	pub fn reading_like(&self, other: &Self) -> Self {
		let mut db = self.clone();
		db.replica.clone_from(&other.replica);
		db
	}

	/// Postgres: where the primary's write-ahead log ends now, as bytes from
	/// its start. Everything committed so far is before it, so a replica
	/// that has replayed up to it shows all of that. `None` on SQLite.
	///
	/// # Errors
	///
	/// When the primary can't be reached.
	pub async fn wal_position(&self) -> Result<Option<i64>> {
		#[cfg(feature = "postgres")]
		if let Some(pool) = self.postgres_pool() {
			let rows = pool
				.fetch(Statement {
					sql: "SELECT (pg_current_wal_insert_lsn() - '0/0'::pg_lsn)::bigint".into(),
					args: Vec::new(),
				})
				.await?;
			return Ok(match rows.first().and_then(|row| row.first()) {
				Some(Value::Int(position)) => Some(*position),
				_ => None,
			});
		}

		Ok(None)
	}

	/// How far the replica this handle reads from has replayed, asking it
	/// now; `None` for a handle on the primary, or a replica that doesn't
	/// answer.
	pub async fn replica_position(&self) -> Option<i64> {
		let replica = self.replica.as_ref()?;
		let position = replica.refresh().await;
		(position != i64::MIN).then_some(position)
	}

	/// Whether this handle reads from a replica.
	pub fn is_on_replica(&self) -> bool {
		self.replica.is_some()
	}

	/// The same handle on a replica that has replayed the primary's
	/// write-ahead log up to `lsn`, as bytes from its start, such as a
	/// change log commit's [`lsn`](crate::live::Commit::lsn): it sees
	/// everything committed before that point. It waits up to `wait` for
	/// one to get there, and returns `None` when none has, or there are
	/// none: then read from the primary.
	pub async fn on_replica_at(&self, lsn: i64, wait: Duration) -> Option<Self> {
		if self.replicas.list.is_empty() {
			return None;
		}

		let replica = self.replicas.at(lsn, wait).await?;
		let mut db = self.clone();
		db.replica = Some(replica);
		Some(db)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[cfg(feature = "postgres")]
	#[tokio::test]
	async fn a_stalled_replica_connection_obeys_the_replay_wait() {
		// The socket accepts TCP connections but never answers Postgres's
		// startup exchange, so the driver's own connection timeout is far
		// longer than the live query's replica wait.
		let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
		let url = format!(
			"postgres://reader@{}/app?sslmode=disable",
			listener.local_addr().unwrap()
		);
		let replica = Replica::connect(&url, crate::PoolOptions::default()).unwrap();
		let replicas = Replicas {
			list: vec![Arc::new(replica)],
			next: AtomicUsize::new(0),
		};
		let selected = tokio::time::timeout(
			Duration::from_secs(1),
			replicas.at(1, Duration::from_millis(25)),
		)
		.await
		.expect("the replica wait also bounds a stalled connection attempt");
		assert!(selected.is_none());
	}

	#[test]
	fn only_selects_go_to_a_replica() {
		assert!(reads_only("SELECT 1"));
		assert!(reads_only("  select * from notes"));
		assert!(!reads_only("INSERT INTO notes VALUES (1) RETURNING *"));
		assert!(!reads_only(
			"WITH gone AS (DELETE FROM notes RETURNING *) SELECT 1"
		));
		assert!(!reads_only("SEL"));
	}

	#[cfg(feature = "postgres")]
	#[test]
	fn names_leave_out_credentials() {
		assert_eq!(
			redact("postgres://reader:secret@db-replica:5432/app?sslmode=require"),
			"postgres://db-replica:5432/app"
		);
		assert_eq!(
			redact("postgres://db-replica/app"),
			"postgres://db-replica/app"
		);
	}
}
