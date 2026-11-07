//! The jobs worker's own paths: configuration and builders, a running job's
//! heartbeat, progress and cancellation, the rows a worker refuses to run,
//! errors too long for their column, `on_failure` sweeps, and the waker.

#![cfg(all(feature = "live", feature = "sqlite"))]
#![allow(
	clippy::cognitive_complexity,
	reason = "each scenario reads as one story, step by step"
)]

use std::collections::BTreeMap;
use std::future::Future;
use std::time::Duration;

use leptoskit_config::DbConfig;
use leptoskit_config::RpcJobsConfig;
use leptoskit_config::RpcQueueConfig;
use leptoskit_db::Db;
use leptoskit_db::Id;
use leptoskit_db::table;
use leptoskit_rpc as rpc;
use leptoskit_rpc::server::Jobs;
use leptoskit_rpc::server::jobs::JobRow;
use leptoskit_rpc::server::jobs::JobState;
use leptoskit_rpc::testing::TestJobs;

/// Reports progress until someone cancels it.
#[rpc::job]
pub async fn worker_slow(cx: &rpc::JobCx) -> rpc::Result<()> {
	for n in 0..100 {
		if cx.cancelled() {
			return Err(rpc::Error::permanent("cancelled"));
		}

		cx.progress(format!("step {n}"));
		cx.progress_fraction(f64::from(n) / 100.0);
		tokio::time::sleep(Duration::from_millis(50)).await;
	}

	Ok(())
}

/// Runs for a while without saying anything about its progress.
#[rpc::job]
pub async fn worker_quiet(_cx: &rpc::JobCx) -> rpc::Result<()> {
	tokio::time::sleep(Duration::from_millis(700)).await;
	Ok(())
}

/// Waits until someone cancels it.
#[rpc::job]
pub async fn worker_hangs(cx: &rpc::JobCx) -> rpc::Result<()> {
	loop {
		if cx.cancelled() {
			return Err(rpc::Error::permanent("cancelled"));
		}

		tokio::time::sleep(Duration::from_millis(50)).await;
	}
}

/// Fails once, with an error longer than its column.
#[rpc::job(retries = 0)]
pub async fn worker_long_error(_cx: &rpc::JobCx) -> rpc::Result<()> {
	let mut error = "a".repeat(1_999);
	error.push('é');
	error.push_str(&"b".repeat(1_000));
	Err(rpc::Error::retry(error))
}

/// Is enqueued to run after its deadline has already passed.
#[rpc::job(deadline = 1s)]
pub async fn worker_deadline(_cx: &rpc::JobCx) -> rpc::Result<()> {
	Ok(())
}

/// Dies inside its only step, after the work and before saving it.
#[rpc::job(retries = 0)]
pub async fn worker_crashes(cx: &rpc::JobCx) -> rpc::Result<()> {
	cx.step("crash", || async { Ok::<_, rpc::Error>(()) })
		.await?;
	Ok(())
}

/// Fails, and its `on_failure` fails in a way no retry fixes.
#[rpc::job(retries = 0, on_failure = worker_failure_permanent)]
pub async fn worker_gives_up(_cx: &rpc::JobCx) -> rpc::Result<()> {
	Err(rpc::Error::retry("no"))
}

#[rpc::mutation]
pub async fn worker_failure_permanent(
	cx: &rpc::MutationCx,
	job: rpc::Failed<WorkerGivesUp>,
) -> rpc::Result<()> {
	let _ = (cx, job);
	Err(rpc::Error::permanent("never"))
}

/// Fails, and its `on_failure` asks to run again.
#[rpc::job(retries = 0, on_failure = worker_failure_retry)]
pub async fn worker_gives_up_again(_cx: &rpc::JobCx) -> rpc::Result<()> {
	Err(rpc::Error::retry("no"))
}

#[rpc::mutation]
pub async fn worker_failure_retry(
	cx: &rpc::MutationCx,
	job: rpc::Failed<WorkerGivesUpAgain>,
) -> rpc::Result<()> {
	let _ = (cx, job);
	Err(rpc::Error::retry("later"))
}

/// A live table, so the waker sees a change that isn't a job.
#[table(name = "worker_marks", live)]
pub struct Mark {
	pub id: Id<Self>,
	pub body: String,
}

/// A job type this build has no registration for, as another instance on a
/// newer build may have enqueued.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct GhostJob;

impl rpc::Job for GhostJob {
	type Output = ();

	const NAME: &'static str = "ghost";
	const OPTIONS: rpc::JobOptions = rpc::JobOptions {
		retries: 0,
		..rpc::JobOptions::DEFAULT
	};

	fn run(self, cx: &rpc::JobCx) -> impl Future<Output = rpc::Result<()>> + Send {
		let _ = cx;
		std::future::ready(Ok(()))
	}
}

/// A job only the `elsewhere` service's workers run, and which fails there:
/// to these workers, one they don't know.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct ElsewhereJob;

impl rpc::Job for ElsewhereJob {
	type Output = ();

	const NAME: &'static str = "elsewhere";
	const OPTIONS: rpc::JobOptions = rpc::JobOptions {
		retries: 0,
		..rpc::JobOptions::DEFAULT
	};
	const SERVICE: Option<&'static str> = Some("elsewhere");

	fn run(self, cx: &rpc::JobCx) -> impl Future<Output = rpc::Result<()>> + Send {
		let _ = cx;
		std::future::ready(Err(rpc::Error::retry("no")))
	}
}

#[allow(deprecated)]
const _: () = {
	rpc::__private::inventory::submit! {
		rpc::server::jobs::JobRegistration {
			name: "elsewhere",
			options: <ElsewhereJob as rpc::Job>::OPTIONS,
			handler: rpc::__private::run_job::<ElsewhereJob>,
			on_failure: None,
			service: Some("elsewhere"),
		}
	}
};

async fn row(db: &Db, id: Id<JobRow>) -> JobRow {
	JobRow::query()
		.filter(|job| job.id.eq(id))
		.one(&db.system("read a job"))
		.await
		.unwrap()
}

/// Waits for a job's row to satisfy `ready`.
async fn wait_row(what: &str, db: &Db, id: Id<JobRow>, ready: impl Fn(&JobRow) -> bool) -> JobRow {
	for _ in 0..1_000 {
		let row = row(db, id).await;

		if ready(&row) {
			return row;
		}

		tokio::time::sleep(Duration::from_millis(10)).await;
	}

	panic!("waited too long for {what}");
}

/// `from_config` and every builder configure the workers they name.
#[tokio::test(flavor = "multi_thread")]
async fn config_and_builders_configure_workers() {
	let db = leptoskit_db::testing::sqlite().await;
	let config = RpcJobsConfig {
		workers: 0,
		poll_seconds: 0,
		retention_days: 0,
		queues: BTreeMap::from([("mail".to_owned(), RpcQueueConfig { concurrency: 0 })]),
	};
	let _ = Jobs::from_config(db.clone(), &config);
	let _ = Jobs::from_configs(db.clone(), &config, &DbConfig::default());
	let _ = Jobs::from_config(
		db.clone(),
		&RpcJobsConfig {
			retention_days: 1,
			..config.clone()
		},
	);

	let jobs = Jobs::from_config(db.clone(), &config)
		.workers(2)
		.concurrency(1)
		.poll(Duration::ZERO)
		.lease(Duration::ZERO)
		.queue("mail", 0)
		.retention(Some(Duration::from_millis(10)))
		.cron(false)
		.live_retention(Some(Duration::from_mins(1)))
		.capture(true)
		.start();
	#[cfg(feature = "postgres")]
	assert!(jobs.capture_stats().is_some(), "capture was asked for");
	jobs.stop();
	jobs.shutdown().await;
}

/// With no `#[rpc::cron]` job in the build, the scheduler has nothing to
/// wait for.
#[tokio::test(flavor = "multi_thread")]
async fn scheduler_without_cron_jobs_returns_immediately() {
	let db = leptoskit_db::testing::sqlite().await;
	let jobs = Jobs::new(db)
		.workers(1)
		.poll(Duration::from_secs(60))
		.cron(true)
		.retention(None)
		.live_retention(None)
		.start();
	tokio::time::sleep(Duration::from_millis(50)).await;
	jobs.shutdown().await;
}

/// A running job writes its progress and learns it was cancelled from its
/// worker's heartbeat.
#[tokio::test(flavor = "multi_thread")]
async fn a_running_job_writes_progress_and_stops_when_cancelled() {
	let db = leptoskit_db::testing::sqlite().await;
	let id = rpc::enqueue(&db, WorkerSlow {}).await.unwrap();
	let jobs = Jobs::new(db.clone())
		.workers(1)
		.poll(Duration::from_millis(10))
		.lease(Duration::from_millis(400))
		.cron(false)
		.retention(None)
		.live_retention(None)
		.start();

	wait_row("the job's first progress", &db, id, |row| {
		row.progress
			.as_deref()
			.is_some_and(|progress| progress.starts_with("step "))
	})
	.await;

	rpc::server::jobs::cancel(&db.system("test"), id)
		.await
		.unwrap();
	wait_row("the cancellation", &db, id, |row| {
		row.state == JobState::Cancelled
	})
	.await;
	jobs.shutdown().await;

	let seen = row(&db, id).await;
	assert_eq!(seen.state, JobState::Cancelled, "{seen:?}");
	assert!(seen.cancel_requested);
	assert_eq!(seen.error.as_deref(), Some("cancelled"));
	assert!(seen.progress.is_some(), "progress was written");
	assert!(seen.fraction.is_some(), "the fraction was written");
}

/// A running job with nothing to say still has its lease renewed.
#[tokio::test(flavor = "multi_thread")]
async fn a_quiet_job_renews_its_lease() {
	let db = leptoskit_db::testing::sqlite().await;
	let id = rpc::enqueue(&db, WorkerQuiet {}).await.unwrap();
	let jobs = Jobs::new(db.clone())
		.workers(1)
		.poll(Duration::from_millis(10))
		.cron(false)
		.retention(None)
		.live_retention(None)
		.start();

	wait_row("the job to finish", &db, id, |row| {
		row.state == JobState::Done
	})
	.await;
	jobs.shutdown().await;
}

/// A worker takes over a job whose lease ran out while it waited to be
/// cancelled, and refuses to run it.
#[tokio::test(flavor = "multi_thread")]
async fn a_cancelled_running_job_is_refused_by_the_next_worker() {
	let db = leptoskit_db::testing::sqlite().await;
	let id = rpc::enqueue(&db, WorkerHangs {}).await.unwrap();
	let jobs = Jobs::new(db.clone())
		.workers(2)
		.poll(Duration::from_millis(10))
		.lease(Duration::from_millis(100))
		.cron(false)
		.retention(None)
		.live_retention(None)
		.start();

	wait_row("the job to start", &db, id, |row| {
		row.state == JobState::Running
	})
	.await;
	rpc::server::jobs::cancel(&db.system("test"), id)
		.await
		.unwrap();
	wait_row("the cancellation", &db, id, |row| {
		row.state == JobState::Cancelled
	})
	.await;
	jobs.shutdown().await;
}

/// A job with no registration here waits, untouched, for workers that know
/// it, and a long error is cut to its column without splitting a character.
#[tokio::test(flavor = "multi_thread")]
async fn unknown_jobs_wait_and_long_errors_are_recorded() {
	let db = leptoskit_db::testing::sqlite().await;
	let jobs = TestJobs::new(&db);

	let unknown = rpc::enqueue(&db, GhostJob).await.unwrap();
	assert_eq!(jobs.run_jobs_until_idle().await, 0);
	jobs.advance(Duration::from_secs(3600)).await;
	assert_eq!(jobs.run_jobs_until_idle().await, 0);
	let waiting = row(&db, unknown).await;
	assert_eq!(waiting.state, JobState::Pending, "{waiting:?}");
	assert_eq!(waiting.attempts, 0, "no attempt was spent on it");

	let long = rpc::enqueue(&db, WorkerLongError {}).await.unwrap();
	jobs.run_jobs_until_idle().await;
	let failed = row(&db, long).await;
	assert_eq!(failed.state, JobState::Failed, "{failed:?}");
	let error = failed.error.unwrap();
	assert_eq!(error.len(), 1_999, "cut on a character boundary");
	assert!(error.bytes().all(|byte| byte == b'a'));
}

/// A job whose deadline passed waits no more, and one whose worker died on
/// its last attempt doesn't run again.
#[tokio::test(flavor = "multi_thread")]
async fn a_deadline_and_an_exhausted_attempt_do_not_run() {
	let db = leptoskit_db::testing::sqlite().await;
	let jobs = TestJobs::new(&db);

	let deadline = rpc::enqueue(&db, WorkerDeadline {}).await.unwrap();
	jobs.advance(Duration::from_secs(2)).await;
	jobs.run_jobs_until_idle().await;
	let seen = row(&db, deadline).await;
	assert_eq!(seen.state, JobState::Failed, "{seen:?}");
	assert_eq!(seen.error.as_deref(), Some("the job passed its deadline"));

	let crashed = rpc::enqueue(&db, WorkerCrashes {}).await.unwrap();
	jobs.kill_in_step("crash");
	assert_eq!(jobs.run_jobs_until_idle().await, 1);
	let running = row(&db, crashed).await;
	assert_eq!(running.state, JobState::Running, "{running:?}");
	assert_eq!(running.attempts, 1);

	jobs.advance(Duration::from_secs(61)).await;
	jobs.run_jobs_until_idle().await;
	let seen = row(&db, crashed).await;
	assert_eq!(seen.state, JobState::Failed, "{seen:?}");
	assert_eq!(seen.attempts, 2);
	assert_eq!(
		seen.error.as_deref(),
		Some("the worker running its last attempt stopped")
	);
}

/// An `on_failure` that can never succeed is not tried again, and one for a
/// job this build doesn't know is simply cleared.
#[tokio::test(flavor = "multi_thread")]
async fn on_failure_sweeps_finish_permanent_and_missing_handlers() {
	let db = leptoskit_db::testing::sqlite().await;
	let jobs = TestJobs::new(&db);

	let id = rpc::enqueue(&db, WorkerGivesUp {}).await.unwrap();
	jobs.run_jobs_until_idle().await;
	let failed = row(&db, id).await;
	assert_eq!(failed.state, JobState::Failed, "{failed:?}");
	assert!(!failed.failure_pending, "the permanent on_failure ran");

	// One that other workers, which know it, failed.
	let missing = rpc::enqueue(&db, ElsewhereJob).await.unwrap();
	TestJobs::new(&db)
		.service(Some("elsewhere"))
		.run_jobs_until_idle()
		.await;
	assert_eq!(row(&db, missing).await.state, JobState::Failed);
	jobs.sweep_again(missing).await;
	assert!(row(&db, missing).await.failure_pending);
	jobs.run_jobs_until_idle().await;
	assert!(!row(&db, missing).await.failure_pending);
}

/// An `on_failure` that asks to run again leaves the job pending, so a
/// later sweep tries it once more.
#[tokio::test(flavor = "multi_thread")]
async fn a_retrying_on_failure_stays_pending() {
	let db = leptoskit_db::testing::sqlite().await;
	let jobs = Jobs::new(db.clone())
		.workers(1)
		.poll(Duration::from_secs(60))
		.cron(false)
		.retention(None)
		.live_retention(None)
		.start();
	let id = rpc::enqueue(&db, WorkerGivesUpAgain {}).await.unwrap();

	wait_row("the job to fail", &db, id, |row| {
		row.state == JobState::Failed
	})
	.await;
	tokio::time::sleep(Duration::from_millis(200)).await;
	jobs.shutdown().await;

	let failed = row(&db, id).await;
	assert_eq!(failed.state, JobState::Failed, "{failed:?}");
	assert!(failed.failure_pending, "the on_failure is still to run");
}

/// The waker ignores changes to other tables and wakes on an invalidation
/// that says changes were missed.
#[tokio::test(flavor = "multi_thread")]
async fn the_waker_ignores_other_tables_and_wakes_on_everything() {
	let db = leptoskit_db::testing::sqlite().await;
	let jobs = Jobs::new(db.clone())
		.workers(1)
		.poll(Duration::from_secs(60))
		.cron(false)
		.retention(None)
		.live_retention(None)
		.start();
	tokio::time::sleep(Duration::from_millis(300)).await;

	NewMark {
		body: "not a job".into(),
	}
	.insert(&db)
	.await
	.unwrap();
	NewMark {
		body: "still not a job".into(),
	}
	.insert(&db)
	.await
	.unwrap();
	tokio::time::sleep(Duration::from_millis(100)).await;
	db.system("test")
		.prune_changes(Duration::ZERO)
		.await
		.unwrap();
	tokio::time::sleep(Duration::from_millis(1_500)).await;
	jobs.shutdown().await;
}

/// A worker whose claim keeps failing waits and tries again rather than
/// stopping the whole set.
#[tokio::test(flavor = "multi_thread")]
async fn a_failing_claim_keeps_the_worker_alive() {
	let db = leptoskit_db::testing::sqlite().await;
	let jobs = Jobs::new(db.clone())
		.workers(1)
		.poll(Duration::from_millis(10))
		.cron(false)
		.retention(None)
		.live_retention(None)
		.start();
	tokio::time::sleep(Duration::from_millis(100)).await;

	{
		use leptoskit_db::Connection as _;
		use leptoskit_db::Statement;

		db.execute(Statement::new("DROP TABLE rpc_jobs", Vec::new()))
			.await
			.unwrap();
	}
	tokio::time::sleep(Duration::from_millis(1_200)).await;
	jobs.shutdown().await;
}
