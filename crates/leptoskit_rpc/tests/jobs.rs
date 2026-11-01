//! Background jobs end to end, on SQLite and Postgres: enqueueing in a
//! mutation's transaction, steps that resume after a crash with the same
//! keys, retries and `on_failure`, deadlines, cancelling, who may see a job,
//! cron, pruning, and real workers that wake on enqueue and keep queue
//! limits and `serialize_by` across two pools.

#![cfg(all(feature = "postgres", feature = "sqlite"))]
#![allow(
	clippy::cognitive_complexity,
	reason = "each scenario reads as one story, step by step"
)]

#[path = "../../leptoskit_db/tests/support/postgres.rs"]
mod postgres;

use std::sync::Mutex;
use std::sync::atomic::AtomicU32;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::time::Duration;

use leptoskit_db::Actor;
use leptoskit_db::Context;
use leptoskit_db::Db;
use leptoskit_db::Id;
use leptoskit_db::table;
use leptoskit_rpc as rpc;
use leptoskit_rpc::server::Jobs;
use leptoskit_rpc::server::jobs::JobRow;
use leptoskit_rpc::server::jobs::JobState;
use leptoskit_rpc::testing::TestJobs;
use postgres::PostgresCluster;

/// A count per name, which jobs' mutations bump.
#[table(name = "tallies")]
pub struct Tally {
	pub id: Id<Self>,
	#[db(unique, max_len = 128)]
	pub name: String,
	pub count: i64,
}

/// Adds one to the tally `name`.
#[rpc::mutation]
pub async fn bump(cx: &rpc::MutationCx, name: String) -> rpc::Result<i64> {
	let existing = Tally::query()
		.filter(|tally| tally.name.eq(name.clone()))
		.first(cx.tx())
		.await?;
	let Some(tally) = existing else {
		NewTally { name, count: 1 }.insert(cx.tx()).await?;

		return Ok(1);
	};
	let count = tally.count + 1;
	Tally::update()
		.filter(|row| row.id.eq(tally.id))
		.set(|row| row.count, count)
		.run(cx.tx())
		.await?;
	Ok(count)
}

async fn tally(db: &Db, name: &str) -> i64 {
	Tally::query()
		.filter(|tally| tally.name.eq(name.to_owned()))
		.first(&db.system("read a tally"))
		.await
		.unwrap()
		.map_or(0, |tally| tally.count)
}

/// What the outside world saw: (tag, what, idempotency key).
static OUTSIDE: Mutex<Vec<(String, String, String)>> = Mutex::new(Vec::new());

fn outside(tag: &str, what: &str, key: String) {
	OUTSIDE
		.lock()
		.unwrap()
		.push((tag.to_owned(), what.to_owned(), key));
}

fn seen(tag: &str) -> Vec<(String, String)> {
	OUTSIDE
		.lock()
		.unwrap()
		.iter()
		.filter(|(seen, ..)| seen == tag)
		.map(|(_, what, key)| (what.clone(), key.clone()))
		.collect()
}

/// Submits, records the submission, then waits for confirmation: the shape
/// of a payment.
#[rpc::job(queue = "chain", retries = 5)]
pub async fn settle(cx: &rpc::JobCx, tag: String) -> rpc::Result<String> {
	let signature = cx
		.step("submit", || {
			async {
				outside(&tag, "submit", cx.key());
				Ok(format!("sig-{tag}"))
			}
		})
		.await?;
	cx.mutate(Bump { name: tag.clone() }).await?;
	cx.progress("Waiting for confirmation");
	cx.progress_fraction(0.5);
	let confirmed = cx
		.step("confirm", || {
			async {
				outside(&tag, "confirm", cx.key());
				Ok::<_, rpc::Error>(true)
			}
		})
		.await?;
	cx.progress_fraction(2.0);
	assert_eq!(cx.step_key("confirm"), format!("{}:confirm", cx.id()));
	Ok(format!("{signature}:{confirmed}"))
}

/// Fails every time, and gives up after three attempts.
#[rpc::job(retries = 2, backoff = exponential(1s, 1s), on_failure = doomed_gave_up)]
pub async fn doomed(cx: &rpc::JobCx, tag: String) -> rpc::Result<()> {
	outside(&tag, "attempt", cx.attempt().to_string());
	Err(rpc::Error::retry("the network is down"))
}

/// Runs once when `doomed` gives up.
#[rpc::mutation]
pub async fn doomed_gave_up(cx: &rpc::MutationCx, job: rpc::Failed<Doomed>) -> rpc::Result<()> {
	bump(cx, format!("{}-gave-up", job.job.tag)).await?;
	assert_eq!(job.attempts, 3);
	assert_eq!(job.error, "the network is down");
	assert_ne!(job.id, "");
	cx.notify(
		cx.identity().context.actor(),
		rpc::Notice::new("It failed").kind(rpc::NoticeKind::Error),
	)
	.await?;
	Ok(())
}

/// Fails in a way no retry fixes.
#[rpc::job(retries = 10)]
pub async fn declined(_cx: &rpc::JobCx, tag: String) -> rpc::Result<()> {
	Err(rpc::Error::permanent(format!("{tag} was declined")))
}

/// Has a minute to succeed, and waits two between attempts.
#[rpc::job(retries = 10, backoff = exponential(2m, 2m), deadline = 1m)]
pub async fn hurried(_cx: &rpc::JobCx, tag: String) -> rpc::Result<()> {
	Err(rpc::Error::retry(format!("{tag} isn't ready")))
}

/// Takes longer than it may.
#[rpc::job(retries = 0, timeout = 50ms)]
pub async fn sluggish(_cx: &rpc::JobCx) -> rpc::Result<()> {
	tokio::time::sleep(Duration::from_secs(5)).await;
	Ok(())
}

/// Cancels itself between its steps.
#[rpc::job]
pub async fn quitter(cx: &rpc::JobCx, tag: String) -> rpc::Result<()> {
	let id = cx.id().parse().unwrap();
	cx.step("first", || {
		async {
			outside(&tag, "first", cx.key());
			rpc::cancel(cx.db(), id).await
		}
	})
	.await?;
	cx.step("second", || {
		async {
			outside(&tag, "second", cx.key());
			Ok(())
		}
	})
	.await
}

/// Waits; nobody should run it in these tests.
#[rpc::job]
pub async fn later(_cx: &rpc::JobCx, tag: String) -> rpc::Result<String> {
	Ok(tag)
}

/// Visible to the account it names, too.
#[rpc::job(visible_to = recipient)]
pub async fn gift(_cx: &rpc::JobCx, recipient: Id<()>) -> rpc::Result<()> {
	let _ = recipient;
	Ok(())
}

/// Visible to anyone.
#[rpc::job(visible_to = everyone)]
pub async fn announcement(_cx: &rpc::JobCx) -> rpc::Result<()> {
	Ok(())
}

/// Visible to anyone signed in.
#[rpc::job(visible_to = signed_in)]
pub async fn bulletin(_cx: &rpc::JobCx) -> rpc::Result<()> {
	Ok(())
}

/// Tells its owner it's done.
#[rpc::job]
pub async fn report(cx: &rpc::JobCx) -> rpc::Result<()> {
	cx.notify(cx.owner(), rpc::Notice::new("Your report is ready"))
		.await?;
	let next = cx
		.enqueue(Later { tag: "next".into() })
		.after(Duration::from_secs(3_600))
		.await?;
	assert_ne!(next.to_string(), "");
	Ok(())
}

/// Enqueues `later` with a key, twice, in one transaction.
#[rpc::mutation]
pub async fn schedule_later(cx: &rpc::MutationCx, tag: String) -> rpc::Result<Id<JobRow>> {
	let first = cx
		.enqueue(Later { tag: tag.clone() })
		.key(format!("later-{tag}"))
		.after(Duration::from_secs(3_600))
		.priority(3)
		.max_attempts(2)
		.queue("slow")
		.await?;
	let again = cx
		.enqueue(Later { tag: tag.clone() })
		.key(format!("later-{tag}"))
		.await?;
	assert_eq!(first, again);
	Ok(first)
}

/// Enqueues `settle` for the caller.
#[rpc::mutation]
pub async fn start_settle(cx: &rpc::MutationCx, tag: String) -> rpc::Result<Id<JobRow>> {
	cx.enqueue(Settle { tag }).await
}

/// Runs every five minutes.
#[rpc::cron("*/5 * * * *")]
#[rpc::job]
pub async fn heartbeat(_cx: &rpc::JobCx) -> rpc::Result<()> {
	Ok(())
}

/// Counts how many run at once in a queue of two.
#[rpc::job(queue = "narrow")]
pub async fn narrow(_cx: &rpc::JobCx, n: u32) -> rpc::Result<()> {
	let _ = n;
	let now = NARROW.fetch_add(1, Ordering::SeqCst) + 1;
	NARROW_MOST.fetch_max(now, Ordering::SeqCst);
	tokio::time::sleep(Duration::from_millis(150)).await;
	NARROW.fetch_sub(1, Ordering::SeqCst);
	NARROW_DONE.fetch_add(1, Ordering::SeqCst);
	Ok(())
}

static NARROW: AtomicUsize = AtomicUsize::new(0);
static NARROW_MOST: AtomicUsize = AtomicUsize::new(0);
static NARROW_DONE: AtomicUsize = AtomicUsize::new(0);

/// One at a time per wallet.
#[rpc::job(serialize_by = wallet)]
pub async fn transfer(_cx: &rpc::JobCx, wallet: String, n: u32) -> rpc::Result<()> {
	let _ = n;
	let slot = if wallet == "a" { &WALLET_A } else { &WALLET_B };
	let most = if wallet == "a" {
		&WALLET_A_MOST
	} else {
		&WALLET_B_MOST
	};

	let now = slot.fetch_add(1, Ordering::SeqCst) + 1;
	most.fetch_max(now, Ordering::SeqCst);
	tokio::time::sleep(Duration::from_millis(100)).await;
	slot.fetch_sub(1, Ordering::SeqCst);

	TRANSFERS.fetch_add(1, Ordering::SeqCst);
	Ok(())
}

static WALLET_A: AtomicUsize = AtomicUsize::new(0);
static WALLET_B: AtomicUsize = AtomicUsize::new(0);
static WALLET_A_MOST: AtomicUsize = AtomicUsize::new(0);
static WALLET_B_MOST: AtomicUsize = AtomicUsize::new(0);
static TRANSFERS: AtomicUsize = AtomicUsize::new(0);

/// Counts its runs, for the wake-up test.
#[rpc::job]
pub async fn ping(_cx: &rpc::JobCx, tag: String) -> rpc::Result<()> {
	outside(&tag, "ping", String::new());
	PINGS.fetch_add(1, Ordering::SeqCst);
	Ok(())
}

static PINGS: AtomicU32 = AtomicU32::new(0);

fn account() -> Id<()> {
	Id::new()
}

fn as_account(db: &Db, account: Id<()>) -> Db {
	db.with_context(Context::new().with(Actor::of(account)))
}

async fn row(db: &Db, id: Id<JobRow>) -> JobRow {
	JobRow::query()
		.filter(|job| job.id.eq(id))
		.one(&db.system("read a job"))
		.await
		.unwrap()
}

async fn steps_resume_after_a_crash_with_the_same_keys(db: &Db) {
	let ada = as_account(db, account());
	let jobs = TestJobs::new(db).lease(Duration::from_secs(30));
	let id = rpc::call(&ada, StartSettle { tag: "pay".into() })
		.await
		.unwrap();

	// The worker dies after confirming, before saving it.
	jobs.kill_in_step("confirm");
	assert_eq!(jobs.run_jobs_until_idle().await, 1);
	let crashed = row(db, id).await;
	assert_eq!(crashed.state, JobState::Running);
	assert!(crashed.steps.0.get("submit").is_some());
	assert!(crashed.steps.0.get("confirm").is_none());
	// Nothing runs until its lease runs out.
	assert_eq!(jobs.run_jobs_until_idle().await, 0);

	jobs.advance(Duration::from_secs(31)).await;
	jobs.run_jobs_until_idle().await;
	let done = row(db, id).await;
	assert_eq!(done.state, JobState::Done, "{done:?}");
	assert_eq!(done.attempts, 2);
	assert_eq!(done.result.unwrap().0, serde_json::json!("sig-pay:true"));
	assert_eq!(done.progress.as_deref(), Some("Waiting for confirmation"));
	assert_eq!(done.fraction, Some(1.0));

	// Submitted once, confirmed twice with the same key, and the mutation
	// between them applied once.
	let seen = seen("pay");
	assert_eq!(seen.len(), 3, "{seen:?}");
	assert_eq!(seen[0], ("submit".into(), format!("{id}:submit")));
	assert_eq!(seen[1], ("confirm".into(), format!("{id}:confirm")));
	assert_eq!(seen[1], seen[2]);
	assert_eq!(tally(db, "pay").await, 1);

	// Its owner reads it; nobody else does.
	let status = rpc::call(&ada, rpc::JobStatusArgs { id: id.to_string() })
		.await
		.unwrap();
	assert_eq!(status.state, "done");
	assert_eq!(status.queue, "chain");
	assert_eq!(status.attempts, 2);
	assert_eq!(status.max_attempts, 6);
	assert!(status.finished_at.is_some());
	let grace = as_account(db, account());
	assert!(matches!(
		rpc::call(&grace, rpc::JobStatusArgs { id: id.to_string() }).await,
		Err(rpc::Error::NotFound(_))
	));

	assert!(matches!(
		rpc::call(&grace, rpc::JobStatusArgs { id: "nope".into() }).await,
		Err(rpc::Error::Invalid(_))
	));
}

async fn failures_retry_then_run_on_failure_once(db: &Db) {
	let ada_id = account();
	let ada = as_account(db, ada_id);
	let jobs = TestJobs::new(db);
	let id = rpc::enqueue(&ada, Doomed { tag: "doom".into() })
		.await
		.unwrap();

	jobs.run_jobs_until_idle().await;
	let waiting = row(db, id).await;
	assert_eq!(waiting.state, JobState::Pending);
	assert_eq!(waiting.error.as_deref(), Some("the network is down"));
	// Its backoff hasn't passed.
	assert_eq!(jobs.run_jobs_until_idle().await, 0);

	for _ in 0..2 {
		jobs.advance(Duration::from_secs(2)).await;
		jobs.run_jobs_until_idle().await;
	}

	let failed = row(db, id).await;
	assert_eq!(failed.state, JobState::Failed, "{failed:?}");
	assert_eq!(failed.attempts, 3);
	assert!(!failed.failure_pending);
	assert_eq!(
		seen("doom")
			.into_iter()
			.map(|(_, attempt)| attempt)
			.collect::<Vec<_>>(),
		["1", "2", "3"]
	);
	assert_eq!(tally(db, "doom-gave-up").await, 1);

	// Swept again, as two instances might: its receipt holds it to once.
	jobs.sweep_again(id).await;
	assert_eq!(jobs.run_jobs_until_idle().await, 1);
	assert_eq!(tally(db, "doom-gave-up").await, 1);
	assert!(!row(db, id).await.failure_pending);

	// The owner heard about it.
	let inbox = rpc::call(&ada, rpc::InboxArgs { limit: 10 }).await.unwrap();
	assert_eq!(inbox.unread, 1);
	assert_eq!(inbox.notices[0].notice.title, "It failed");
	assert_eq!(inbox.notices[0].notice.kind, rpc::NoticeKind::Error);

	// An operator runs it again from the start.
	rpc::server::jobs::retry(&db.system("test"), id)
		.await
		.unwrap();
	let again = row(db, id).await;
	assert_eq!((again.state, again.attempts), (JobState::Pending, 0));
	assert!(matches!(
		rpc::server::jobs::retry(&db.system("test"), id).await,
		Err(rpc::Error::Conflict(_))
	));
	assert!(matches!(
		rpc::server::jobs::retry(&db.system("test"), Id::new()).await,
		Err(rpc::Error::NotFound(_))
	));
}

async fn permanent_failures_deadlines_and_timeouts_stop_at_once(db: &Db) {
	let jobs = TestJobs::new(db);
	let declined = rpc::enqueue(db, Declined { tag: "card".into() })
		.await
		.unwrap();
	let hurried = rpc::enqueue(db, Hurried { tag: "late".into() })
		.await
		.unwrap();
	let sluggish = rpc::enqueue(db, Sluggish {}).await.unwrap();
	jobs.run_jobs_until_idle().await;

	let declined = row(db, declined).await;
	assert_eq!((declined.state, declined.attempts), (JobState::Failed, 1));
	assert_eq!(declined.error.as_deref(), Some("card was declined"));
	assert!(!declined.failure_pending);

	// Its next attempt would come after its deadline.
	let hurried_row = row(db, hurried).await;
	assert_eq!(hurried_row.state, JobState::Failed);
	assert_eq!(hurried_row.attempts, 1);

	let sluggish = row(db, sluggish).await;
	assert_eq!(sluggish.state, JobState::Failed);
	assert!(sluggish.error.unwrap().contains("timeout"));

	// A deadline that passed while it waited fails it before it runs.
	let waited = rpc::enqueue(
		db,
		Hurried {
			tag: "waited".into(),
		},
	)
	.after(Duration::from_secs(120))
	.await
	.unwrap();
	jobs.advance(Duration::from_secs(121)).await;
	jobs.run_jobs_until_idle().await;
	let waited = row(db, waited).await;
	assert_eq!(waited.state, JobState::Failed);
	assert_eq!(waited.error.as_deref(), Some("the job passed its deadline"));
}

async fn cancelling_stops_waiting_and_running_jobs(db: &Db) {
	let ada_id = account();
	let ada = as_account(db, ada_id);
	let grace = as_account(db, account());
	let jobs = TestJobs::new(db);

	let waiting = rpc::call(&ada, ScheduleLater { tag: "w".into() })
		.await
		.unwrap();
	let pending = row(db, waiting).await;
	assert_eq!(pending.queue, "slow");
	assert_eq!((pending.priority, pending.max_attempts), (3, 2));
	assert_eq!(pending.owner, ada_id.to_string());
	// Somebody else can't see it, so can't cancel it.
	assert!(matches!(
		rpc::call(
			&grace,
			rpc::CancelJob {
				id: waiting.to_string()
			}
		)
		.await,
		Err(rpc::Error::NotFound(_))
	));
	rpc::call(
		&ada,
		rpc::CancelJob {
			id: waiting.to_string(),
		},
	)
	.await
	.unwrap();
	assert_eq!(row(db, waiting).await.state, JobState::Cancelled);

	// Cancelling a finished job changes nothing.
	rpc::call(
		&ada,
		rpc::CancelJob {
			id: waiting.to_string(),
		},
	)
	.await
	.unwrap();
	jobs.advance(Duration::from_secs(7_200)).await;
	jobs.run_jobs_until_idle().await;
	assert_eq!(row(db, waiting).await.state, JobState::Cancelled);

	// A running job stops at its next step.
	let running = rpc::enqueue(&ada, Quitter { tag: "quit".into() })
		.await
		.unwrap();
	jobs.run_jobs_until_idle().await;
	let stopped = row(db, running).await;
	assert_eq!(stopped.state, JobState::Cancelled, "{stopped:?}");
	assert!(stopped.cancel_requested);
	assert_eq!(
		seen("quit")
			.into_iter()
			.map(|(what, _)| what)
			.collect::<Vec<_>>(),
		["first"]
	);
}

async fn who_may_see_a_job(db: &Db) {
	let ada_id = account();
	let grace_id = account();
	let ada = as_account(db, ada_id);
	let grace = as_account(db, grace_id);
	let nobody = db.with_context(Context::new());
	let visible = async |db: &Db, id: Id<JobRow>| {
		rpc::call(db, rpc::JobStatusArgs { id: id.to_string() })
			.await
			.is_ok()
	};

	let gift = rpc::enqueue(
		&ada,
		Gift {
			recipient: grace_id,
		},
	)
	.await
	.unwrap();
	assert!(visible(&ada, gift).await);
	assert!(visible(&grace, gift).await);
	assert!(!visible(&as_account(db, account()), gift).await);

	let announcement = rpc::enqueue(&ada, Announcement {}).await.unwrap();
	assert!(visible(&nobody, announcement).await);

	let bulletin = rpc::enqueue(&ada, Bulletin {}).await.unwrap();
	assert!(visible(&grace, bulletin).await);
	assert!(!visible(&nobody, bulletin).await);

	// The recipient may read it, but only the owner may cancel it.
	assert!(matches!(
		rpc::call(
			&grace,
			rpc::CancelJob {
				id: gift.to_string()
			}
		)
		.await,
		Err(rpc::Error::Forbidden(_))
	));
	// A job enqueued by nobody signed in is nobody's to read.
	let orphan = rpc::enqueue(&nobody, Later { tag: "o".into() })
		.after(Duration::from_secs(60))
		.await
		.unwrap();
	assert!(!visible(&nobody, orphan).await);
	assert_eq!(row(db, orphan).await.owner, "");
}

async fn notices_reach_their_owner_and_enqueued_jobs_inherit_it(db: &Db) {
	let ada = as_account(db, account());
	let jobs = TestJobs::new(db);
	rpc::enqueue(&ada, Report {}).await.unwrap();
	jobs.run_jobs_until_idle().await;
	let inbox = rpc::call(&ada, rpc::InboxArgs { limit: 10 }).await.unwrap();
	assert_eq!(inbox.notices.len(), 1);
	assert_eq!(inbox.notices[0].notice.title, "Your report is ready");
	let next = jobs
		.all()
		.await
		.into_iter()
		.find(|job| job.name == "later")
		.unwrap();

	assert_eq!(next.owner, row(db, next.id).await.owner);
	assert_ne!(next.owner, "");
}

async fn cron_enqueues_each_tick_once_across_schedulers(db: &Db) {
	let first = TestJobs::new(db);
	let second = TestJobs::new(db);
	first.advance(Duration::from_secs(600)).await;
	second.advance(Duration::from_secs(600)).await;
	let ticks: Vec<JobRow> = first
		.all()
		.await
		.into_iter()
		.filter(|job| job.name == "heartbeat")
		.collect();
	assert_eq!(ticks.len(), 2, "{ticks:?}");
	assert!(ticks.iter().all(|job| {
		job.key
			.as_deref()
			.is_some_and(|key| key.starts_with("cron:heartbeat:"))
	}));
	first.run_jobs_until_idle().await;

	assert!(
		first
			.all()
			.await
			.iter()
			.filter(|job| job.name == "heartbeat")
			.all(|job| job.state == JobState::Done)
	);

	// Finished jobs go after their retention; failed ones stay.
	let kept = rpc::enqueue(db, Declined { tag: "kept".into() })
		.await
		.unwrap();
	first.run_jobs_until_idle().await;
	first.advance(Duration::from_secs(2)).await;
	assert_eq!(first.prune(Duration::from_secs(1)).await, 2);
	assert!(
		first
			.all()
			.await
			.iter()
			.all(|job| job.state != JobState::Done)
	);
	assert_eq!(row(db, kept).await.state, JobState::Failed);
	assert_eq!(
		rpc::server::jobs::prune(db, Duration::from_secs(3_600))
			.await
			.unwrap(),
		0
	);
}

async fn a_job_called_directly_runs_inline(db: &Db) {
	let ada = as_account(db, account());
	let output = rpc::call(
		&ada,
		Settle {
			tag: "inline".into(),
		},
	)
	.await
	.unwrap();
	assert_eq!(output, "sig-inline:true");
	assert_eq!(tally(db, "inline").await, 1);
}

async fn workers_wake_on_enqueue(enqueue_on: &Db, work_on: &Db, tag: &str) {
	// A long poll: only the wake-up can make it this quick.
	let jobs = Jobs::new(work_on.clone())
		.workers(1)
		.poll(Duration::from_secs(60))
		.start();
	// Let the waker subscribe.
	tokio::time::sleep(Duration::from_millis(300)).await;
	rpc::enqueue(enqueue_on, Ping { tag: tag.into() })
		.await
		.unwrap();
	let until = std::time::Instant::now() + Duration::from_secs(10);

	while seen(tag).is_empty() {
		assert!(std::time::Instant::now() < until, "the worker wasn't woken");
		tokio::time::sleep(Duration::from_millis(20)).await;
	}

	jobs.shutdown().await;
}

async fn limits_hold_across_two_pools(first: &Db, second: &Db) {
	let pools = [first, second].map(|db| {
		Jobs::new(db.clone())
			.workers(4)
			.queue("narrow", 2)
			.poll(Duration::from_millis(50))
			.lease(Duration::from_secs(30))
			.cron(false)
			.retention(None)
			.start()
	});
	NARROW_MOST.store(0, Ordering::SeqCst);
	NARROW_DONE.store(0, Ordering::SeqCst);
	TRANSFERS.store(0, Ordering::SeqCst);
	WALLET_A_MOST.store(0, Ordering::SeqCst);
	WALLET_B_MOST.store(0, Ordering::SeqCst);

	for n in 0..8 {
		rpc::enqueue(first, Narrow { n }).await.unwrap();
	}

	for n in 0..4 {
		rpc::enqueue(
			first,
			Transfer {
				wallet: "a".into(),
				n,
			},
		)
		.await
		.unwrap();
		rpc::enqueue(
			second,
			Transfer {
				wallet: "b".into(),
				n,
			},
		)
		.await
		.unwrap();
	}

	let until = std::time::Instant::now() + Duration::from_secs(60);

	while NARROW_DONE.load(Ordering::SeqCst) < 8 || TRANSFERS.load(Ordering::SeqCst) < 8 {
		assert!(std::time::Instant::now() < until, "the jobs didn't finish");
		tokio::time::sleep(Duration::from_millis(50)).await;
	}

	for pool in pools {
		pool.shutdown().await;
	}

	assert!(NARROW_MOST.load(Ordering::SeqCst) <= 2);
	assert!(NARROW_MOST.load(Ordering::SeqCst) >= 1);
	assert_eq!(WALLET_A_MOST.load(Ordering::SeqCst), 1);
	assert_eq!(WALLET_B_MOST.load(Ordering::SeqCst), 1);
}

async fn scenarios(db: &Db) {
	steps_resume_after_a_crash_with_the_same_keys(db).await;
	failures_retry_then_run_on_failure_once(db).await;
	permanent_failures_deadlines_and_timeouts_stop_at_once(db).await;
	cancelling_stops_waiting_and_running_jobs(db).await;
	who_may_see_a_job(db).await;
	notices_reach_their_owner_and_enqueued_jobs_inherit_it(db).await;
	a_job_called_directly_runs_inline(db).await;
}

/// The tests share outside-world tags and counters, so they run one at a
/// time.
static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[tokio::test(flavor = "multi_thread")]
async fn sqlite_jobs() {
	let _serial = SERIAL.lock().await;
	OUTSIDE.lock().unwrap().clear();
	scenarios(&leptoskit_db::testing::sqlite().await).await;
	cron_enqueues_each_tick_once_across_schedulers(&leptoskit_db::testing::sqlite().await).await;
	let db = leptoskit_db::testing::sqlite().await;
	workers_wake_on_enqueue(&db, &db, "wake-sqlite").await;
	limits_hold_across_two_pools(&db, &db).await;
}

async fn postgres(cluster: &PostgresCluster) -> (Db, String) {
	let url = cluster.database();
	let db = Db::connect(&url).await.unwrap();
	leptoskit_db::testing::create_tables(&db).await.unwrap();
	(db, url)
}

#[tokio::test(flavor = "multi_thread")]
async fn postgres_jobs() {
	let _serial = SERIAL.lock().await;
	OUTSIDE.lock().unwrap().clear();
	let Some(cluster) = PostgresCluster::start() else {
		return;
	};
	scenarios(&postgres(&cluster).await.0).await;
	cron_enqueues_each_tick_once_across_schedulers(&postgres(&cluster).await.0).await;

	// Two instances: separate pools on one database.
	let (first, url) = postgres(&cluster).await;
	let second = Db::connect(&url).await.unwrap();
	workers_wake_on_enqueue(&first, &second, "wake-postgres").await;
	limits_hold_across_two_pools(&first, &second).await;
}
