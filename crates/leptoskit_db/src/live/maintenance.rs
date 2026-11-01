//! Keeping the change log in shape on a schedule: pruning it, and on
//! Postgres making its partitions ahead of the rows being written.

use std::time::Duration;

use leptoskit_db_schema::Dialect;
use tokio::sync::watch;

use crate::Db;
use crate::Result;

/// The change log's upkeep, running every interval until it's stopped or
/// dropped: what [`Db::live_maintenance`] starts.
pub struct Maintenance {
	stop: watch::Sender<bool>,
	task: Option<tokio::task::JoinHandle<()>>,
}

impl Maintenance {
	/// Asks it to stop, after the round it's in.
	pub fn stop(&self) {
		self.stop.send_replace(true);
	}

	/// Stops it and waits until it has.
	pub async fn shutdown(mut self) {
		self.stop();

		if let Some(task) = self.task.take() {
			let _ = task.await;
		}
	}
}

impl Drop for Maintenance {
	fn drop(&mut self) {
		self.stop();
	}
}

impl std::fmt::Debug for Maintenance {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("Maintenance").finish_non_exhaustive()
	}
}

/// What one server remembers between rounds: how fast each partitioned
/// table grows.
#[derive(Default)]
struct Rounds {
	#[cfg(feature = "postgres")]
	commits: super::partitions::Growth,
	#[cfg(feature = "postgres")]
	changes: super::partitions::Growth,
}

impl Db {
	/// Starts the change log's upkeep, now and then every `interval`: it
	/// prunes commits older than `retention`, and on Postgres makes the log's
	/// partitions ahead of the rows being written, so pruning drops whole
	/// partitions instead of deleting rows. Run it on every server: they
	/// take turns, under an advisory lock on Postgres, and a server that
	/// finds another at it skips that round.
	///
	/// ```ignore
	/// let upkeep = db.live_maintenance(Duration::from_secs(15 * 60), Duration::from_secs(60));
	/// // …and at shutdown:
	/// upkeep.shutdown().await;
	/// ```
	///
	/// A round that fails says why on standard error and is tried again
	/// next time.
	pub fn live_maintenance(&self, retention: Duration, interval: Duration) -> Maintenance {
		let (stop, stopped) = watch::channel(false);
		let task = tokio::spawn(run(
			self.system("maintain the change log"),
			retention,
			interval.max(Duration::from_secs(1)),
			stopped,
		));

		Maintenance {
			stop,
			task: Some(task),
		}
	}

	/// One round of [`Db::live_maintenance`], now: makes partitions ahead on
	/// Postgres, then prunes commits older than `retention`. Returns how many
	/// commits it removed.
	pub async fn maintain_changes(&self, retention: Duration) -> Result<u64> {
		round(self, retention, &mut Rounds::default()).await
	}
}

async fn run(db: Db, retention: Duration, interval: Duration, mut stop: watch::Receiver<bool>) {
	let mut rounds = Rounds::default();

	loop {
		if let Err(error) = round(&db, retention, &mut rounds).await {
			eprintln!("warning: change log maintenance: {error}; trying again in {interval:?}");
		}

		// Asked to stop, or every handle to it dropped.
		if tokio::time::timeout(interval, stop.changed()).await.is_ok() || *stop.borrow() {
			return;
		}
	}
}

#[cfg_attr(
	not(feature = "postgres"),
	expect(
		clippy::needless_pass_by_ref_mut,
		reason = "only Postgres remembers anything between rounds"
	)
)]
async fn round(db: &Db, retention: Duration, rounds: &mut Rounds) -> Result<u64> {
	#[cfg(feature = "postgres")]
	if db.dialect() == Dialect::Postgres
		&& super::log::has_log(db).await?
		&& super::partitions::partitioned(db).await?
	{
		use super::partitions::Log;

		super::partitions::make_ahead(db, Log::Commits, retention, &mut rounds.commits).await?;
		super::partitions::make_ahead(db, Log::Changes, retention, &mut rounds.changes).await?;
		return super::partitions::prune(db, retention).await;
	}

	let _ = (rounds, Dialect::Sqlite);
	super::log::prune(db, retention).await
}
