//! Running background jobs in tests, on a clock the test moves.
//!
//! ```ignore
//! let db = leptoskit::db::testing::sqlite().await;
//! let jobs = rpc::testing::TestJobs::new(&db);
//!
//! rpc::call(&db.with_context(ada), SendPayment { … }).await?;
//! jobs.run_jobs_until_idle().await;          // runs what is due now
//! jobs.advance(Duration::from_secs(60));     // a minute later: retries, `.after(…)`, cron
//! jobs.run_jobs_until_idle().await;
//! ```
//!
//! Jobs run one at a time, on the test's own task, so a test sees each
//! one's effects as soon as `run_jobs_until_idle` returns.
//! [`kill_in_step`](TestJobs::kill_in_step) stops a job inside a step, as a
//! crashed worker would, so a test can check the step runs again with the
//! same key once the lease runs out.

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::PoisonError;
use std::time::Duration;

use leptoskit_db::Db;
use leptoskit_db::Timestamp;

use crate::server::jobs::Clock;
use crate::server::jobs::Faults;
use crate::server::jobs::JobRow;
use crate::server::jobs::Settings;

/// Background jobs for a test: a clock that moves only when the test says,
/// and workers that run only when it asks.
pub struct TestJobs {
	db: Db,
	settings: Settings,
	faults: Arc<Faults>,
	cron_from: Mutex<Timestamp>,
}

impl TestJobs {
	/// Jobs on `db`, starting at the present, with a one-minute lease.
	pub fn new(db: &Db) -> Self {
		let clock = Clock::manual();
		let faults = Arc::new(Faults::default());
		let mut settings = Settings::new(clock.clone());
		settings.faults = Some(Arc::clone(&faults));
		Self {
			db: db.clone(),
			cron_from: Mutex::new(clock.now()),
			settings,
			faults,
		}
	}

	/// How long a claimed job is held before another worker may take it
	/// over.
	#[must_use]
	pub fn lease(mut self, lease: Duration) -> Self {
		self.settings.lease = lease;
		self
	}

	/// Runs at most `concurrency` jobs of `queue` at once.
	#[must_use]
	pub fn queue(mut self, queue: impl Into<String>, concurrency: usize) -> Self {
		Arc::make_mut(&mut self.settings.queues).insert(queue.into(), concurrency.max(1));
		self
	}

	/// The test's time now.
	pub fn now(&self) -> Timestamp {
		self.settings.clock.now()
	}

	/// Moves the clock forward by `by`, and enqueues the `#[rpc::cron]`
	/// jobs whose ticks it passed.
	///
	/// # Panics
	///
	/// If enqueuing a cron job fails.
	pub async fn advance(&self, by: Duration) {
		self.settings.clock.advance(by);
		let until = self.now();
		let from = *self
			.cron_from
			.lock()
			.unwrap_or_else(PoisonError::into_inner);
		crate::server::jobs::enqueue_cron(&self.db, from, until)
			.await
			.expect("cron jobs enqueue");
		*self
			.cron_from
			.lock()
			.unwrap_or_else(PoisonError::into_inner) = until;
	}

	/// Runs every job that is due, one at a time, then the `on_failure`
	/// mutations of jobs that gave up, until nothing is left to do now. How
	/// many runs it made.
	///
	/// # Panics
	///
	/// If the database fails.
	pub async fn run_jobs_until_idle(&self) -> usize {
		let mut runs = 0;

		loop {
			if crate::server::jobs::run_one(&self.db, &self.settings, false)
				.await
				.expect("jobs claim and run")
			{
				runs += 1;
				continue;
			}
			let swept = crate::server::jobs::sweep_failures(&self.db)
				.await
				.expect("on_failure runs");

			if swept == 0 {
				return runs;
			}

			runs += swept;
		}
	}

	/// Kills the worker that next runs step `name`, after the step's work
	/// and before its result is saved, as if the process died there. The
	/// job stays leased: [`advance`](Self::advance) past the lease and it
	/// runs again from that step.
	pub fn kill_in_step(&self, name: &str) {
		self.faults.kill_in(name);
	}

	/// Marks job `id`'s `on_failure` as not run yet, as a second instance
	/// sweeping at the same moment would find it, so the next
	/// [`run_jobs_until_idle`](Self::run_jobs_until_idle) runs it again: its
	/// receipt should keep its effects to once.
	///
	/// # Panics
	///
	/// If the database fails.
	pub async fn sweep_again(&self, id: leptoskit_db::Id<JobRow>) {
		JobRow::update()
			.filter(|job| job.id.eq(id) & job.state.eq(crate::server::jobs::JobState::Failed))
			.set(|job| job.failure_pending, true)
			.run(&self.db.system("sweep a job again in a test"))
			.await
			.expect("the job updates");
	}

	/// Deletes finished jobs older than `age`, by the test's clock.
	///
	/// # Panics
	///
	/// If the database fails.
	pub async fn prune(&self, age: Duration) -> u64 {
		crate::server::jobs::worker_prune_at(&self.db, self.now(), age)
			.await
			.expect("finished jobs prune")
	}

	/// Every job, oldest first.
	///
	/// # Panics
	///
	/// If the database fails.
	pub async fn all(&self) -> Vec<JobRow> {
		JobRow::query()
			.order_by(|job| job.created_at.asc())
			.all(&self.db.system("read jobs in a test"))
			.await
			.expect("jobs read")
	}
}
