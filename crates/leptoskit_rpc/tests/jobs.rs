//! Background jobs end to end: a mutation enqueues in its transaction, a
//! worker claims and runs, a failure retries, and a client reads the state.

#![cfg(all(feature = "sqlite", feature = "server"))]

use std::sync::atomic::AtomicU32;
use std::sync::atomic::Ordering;
use std::time::Duration;

use leptoskit_db::Db;
use leptoskit_rpc as rpc;
use leptoskit_rpc::server::Jobs;

static RAN: AtomicU32 = AtomicU32::new(0);
static FLAKY: AtomicU32 = AtomicU32::new(0);

/// Counts its runs.
#[rpc::job]
pub async fn count(_cx: &rpc::JobCx) -> rpc::Result<()> {
	RAN.fetch_add(1, Ordering::Relaxed);
	Ok(())
}

/// Fails twice, then succeeds.
#[rpc::job]
pub async fn flaky(_cx: &rpc::JobCx) -> rpc::Result<String> {
	let attempt = FLAKY.fetch_add(1, Ordering::Relaxed) + 1;
	if attempt < 3 {
		return Err(rpc::Error::server("not yet"));
	}
	Ok("done".to_owned())
}

/// A mutation that enqueues the counting job.
#[rpc::mutation]
pub async fn schedule(cx: &rpc::MutationCx) -> rpc::Result<()> {
	cx.enqueue(Count {}).await?;
	Ok(())
}

/// A mutation that enqueues the flaky job.
#[rpc::mutation]
pub async fn schedule_flaky(cx: &rpc::MutationCx) -> rpc::Result<()> {
	cx.enqueue(Flaky {}).await?;
	Ok(())
}

async fn wait_for(state: &str, db: &Db, timeout: Duration) {
	let system = db.system("test");
	let until = std::time::Instant::now() + timeout;
	loop {
		let jobs = rpc::server::jobs::JobRow::query()
			.all(&system)
			.await
			.unwrap();
		if jobs
			.iter()
			.any(|job| format!("{:?}", job.state).to_lowercase() == state)
		{
			return;
		}
		assert!(
			std::time::Instant::now() <= until,
			"no job reached {state}: {jobs:?}"
		);
		tokio::time::sleep(Duration::from_millis(20)).await;
	}
}

#[tokio::test(flavor = "multi_thread")]
async fn a_mutation_enqueues_a_job_a_worker_runs() {
	let db = leptoskit_db::testing::sqlite().await;
	let jobs = Jobs::new(db.clone())
		.concurrency(2)
		.lease(Duration::from_secs(5))
		.start();

	rpc::call(&db, Schedule {}).await.unwrap();
	wait_for("done", &db, Duration::from_secs(10)).await;
	assert_eq!(RAN.load(Ordering::Relaxed), 1);

	jobs.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failing_job_retries_with_backoff() {
	let db = leptoskit_db::testing::sqlite().await;
	let jobs = Jobs::new(db.clone())
		.concurrency(1)
		.lease(Duration::from_secs(5))
		.start();

	FLAKY.store(0, Ordering::Relaxed);
	rpc::call(&db, ScheduleFlaky {}).await.unwrap();
	wait_for("done", &db, Duration::from_secs(30)).await;
	assert_eq!(FLAKY.load(Ordering::Relaxed), 3);

	jobs.shutdown().await;
}
