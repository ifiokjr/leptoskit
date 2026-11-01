//! The workers: claiming jobs, running them on a lease, recording how they
//! went, and the chores around them: `on_failure`, cron, pruning, and
//! waking up when a job is enqueued.

use std::collections::HashMap;
use std::collections::HashSet;
use std::hash::BuildHasher;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::PoisonError;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicI64;
use std::sync::atomic::Ordering;
use std::time::Duration;

use leptoskit_db::Actor;
use leptoskit_db::Connection;
use leptoskit_db::Db;
use leptoskit_db::DbError;
use leptoskit_db::Json;
use leptoskit_db::Table;
use leptoskit_db::Timestamp;
use tokio::sync::Notify;

use super::FailureRun;
use super::JobCx;
use super::JobRegistration;
use super::JobRow;
use super::JobRun;
use super::JobState;
use super::cron::CronRegistration;
use super::cx::Progress;
use super::cx::RunState;
use super::duration;
use super::job_registration;
use super::owner_actor;
use super::owner_context;
use crate::Error;
use crate::server::Identity;

/// The time workers go by: the system clock, or, in tests, the system clock
/// moved forward by hand.
#[derive(Clone, Default)]
pub(crate) struct Clock {
	offset: Option<Arc<AtomicI64>>,
}

impl Clock {
	/// A clock tests move forward.
	pub(crate) fn manual() -> Self {
		Self {
			offset: Some(Arc::default()),
		}
	}

	pub(crate) fn now(&self) -> Timestamp {
		let offset = self
			.offset
			.as_ref()
			.map_or(0, |offset| offset.load(Ordering::SeqCst));
		leptoskit_db::now() + chrono::Duration::milliseconds(offset)
	}

	pub(crate) fn advance(&self, by: Duration) {
		if let Some(offset) = &self.offset {
			offset.fetch_add(
				i64::try_from(by.as_millis()).unwrap_or(i64::MAX),
				Ordering::SeqCst,
			);
		}
	}
}

/// Faults a test injects: a worker that dies inside a step.
#[derive(Default)]
pub(crate) struct Faults {
	kill_in: Mutex<HashSet<String>>,
}

impl Faults {
	/// Kills the next run of step `name` after its work, before its result
	/// is saved.
	pub(crate) fn kill_in(&self, name: &str) {
		self.kill_in
			.lock()
			.unwrap_or_else(PoisonError::into_inner)
			.insert(name.to_owned());
	}

	/// Whether step `name` dies now; each fault fires once.
	pub(crate) fn kills(&self, name: &str) -> bool {
		self.kill_in
			.lock()
			.unwrap_or_else(PoisonError::into_inner)
			.remove(name)
	}
}

/// How workers claim and run jobs.
#[derive(Clone)]
pub(crate) struct Settings {
	/// How long a worker holds a job it isn't heard from.
	pub lease: Duration,
	/// Each queue's concurrency, across every instance; queues not named
	/// have none but the workers'.
	pub queues: Arc<HashMap<String, usize>>,
	pub clock: Clock,
	pub faults: Option<Arc<Faults>>,
}

impl Settings {
	pub(crate) fn new(clock: Clock) -> Self {
		Self {
			lease: Duration::from_secs(60),
			queues: Arc::default(),
			clock,
			faults: None,
		}
	}
}

/// How long writes of a job's progress are spread apart.
const PROGRESS_EVERY: Duration = Duration::from_millis(250);

/// The workers that run a database's jobs.
///
/// ```ignore
/// let jobs = rpc::server::Jobs::new(db.clone())
/// 	.workers(4)
/// 	.queue("chain", 2)
/// 	.retention(Duration::from_secs(7 * 24 * 60 * 60))
/// 	.start();
/// // …and at shutdown, after the server stops taking requests:
/// jobs.shutdown().await;
/// ```
pub struct Jobs {
	db: Db,
	workers: usize,
	poll: Duration,
	retention: Option<Duration>,
	cron: bool,
	settings: Settings,
}

impl Jobs {
	/// Workers for `db`'s jobs: four at a time, checking every five seconds
	/// as well as whenever a job is enqueued, with a one-minute lease each,
	/// keeping finished jobs for a week, and enqueuing `#[rpc::cron]` jobs.
	pub fn new(db: Db) -> Self {
		Self {
			db,
			workers: 4,
			poll: Duration::from_secs(5),
			retention: Some(Duration::from_hours(168)),
			cron: true,
			settings: Settings::new(Clock::default()),
		}
	}

	/// Workers configured from `[rpc.jobs]` in `leptoskit.toml`.
	///
	/// ```ignore
	/// let config = LeptoskitConfig::from_toml(include_str!("../../leptoskit.toml"), Path::new("my_app"))?;
	/// let jobs = rpc::server::Jobs::from_config(db.clone(), &config.rpc.jobs).start();
	/// ```
	pub fn from_config(db: Db, config: &leptoskit_config::RpcJobsConfig) -> Self {
		let mut jobs = Self::new(db)
			.workers(usize::try_from(config.workers).unwrap_or(usize::MAX))
			.poll(Duration::from_secs(config.poll_seconds.into()))
			.retention(
				(config.retention_days > 0)
					.then(|| Duration::from_secs(u64::from(config.retention_days) * 24 * 60 * 60)),
			);

		for (name, queue) in &config.queues {
			jobs = jobs.queue(
				name,
				usize::try_from(queue.concurrency).unwrap_or(usize::MAX),
			);
		}

		jobs
	}

	/// How many jobs this instance runs at once.
	#[must_use]
	pub fn workers(mut self, workers: usize) -> Self {
		self.workers = workers.max(1);
		self
	}

	/// The same as [`workers`](Self::workers).
	#[must_use]
	pub fn concurrency(self, concurrency: usize) -> Self {
		self.workers(concurrency)
	}

	/// How often idle workers look for jobs that are due, besides being
	/// woken whenever one is enqueued, on any instance.
	#[must_use]
	pub fn poll(mut self, poll: Duration) -> Self {
		self.poll = poll.max(Duration::from_millis(10));
		self
	}

	/// How long a worker holds a job before another may take it over. A
	/// running job renews its lease as it goes, so this is how soon a job
	/// whose worker died runs again. A job's own `timeout` lengthens it.
	#[must_use]
	pub fn lease(mut self, lease: Duration) -> Self {
		self.settings.lease = lease.max(Duration::from_millis(100));
		self
	}

	/// Runs at most `concurrency` jobs of `queue` at once, across every
	/// instance that runs workers for the same database. Give each instance
	/// the same queues.
	#[must_use]
	pub fn queue(mut self, queue: impl Into<String>, concurrency: usize) -> Self {
		Arc::make_mut(&mut self.settings.queues).insert(queue.into(), concurrency.max(1));
		self
	}

	/// How long finished and cancelled jobs are kept; `None` keeps them.
	/// Failed jobs stay until someone retries them.
	#[must_use]
	pub fn retention(mut self, retention: Option<Duration>) -> Self {
		self.retention = retention;
		self
	}

	/// Whether this instance enqueues `#[rpc::cron]` jobs. Every instance
	/// may: each tick's job is enqueued once, whichever gets there first.
	#[must_use]
	pub fn cron(mut self, cron: bool) -> Self {
		self.cron = cron;
		self
	}

	/// Starts the workers, which run until the returned handle stops them.
	pub fn start(self) -> JobsHandle {
		let shutdown = Arc::new(AtomicBool::new(false));
		let wake = Arc::new(Notify::new());
		let mut tasks = Vec::with_capacity(self.workers + 2);

		tasks.push(tokio::spawn(waker(
			self.db.clone(),
			Arc::clone(&wake),
			Arc::clone(&shutdown),
		)));

		if self.cron {
			tasks.push(tokio::spawn(scheduler(
				self.db.clone(),
				self.settings.clock.clone(),
				Arc::clone(&wake),
				Arc::clone(&shutdown),
			)));
		}

		for index in 0..self.workers {
			tasks.push(tokio::spawn(worker(
				self.db.clone(),
				self.settings.clone(),
				Arc::clone(&shutdown),
				Arc::clone(&wake),
				self.poll,
				// One worker prunes, so instances don't race each other more
				// than they must.
				self.retention.filter(|_| index == 0),
			)));
		}

		JobsHandle {
			shutdown,
			wake,
			tasks,
		}
	}
}

/// A running set of workers.
pub struct JobsHandle {
	shutdown: Arc<AtomicBool>,
	wake: Arc<Notify>,
	tasks: Vec<tokio::task::JoinHandle<()>>,
}

impl JobsHandle {
	/// Asks the workers to stop: they take no more jobs, and finish the
	/// ones they hold.
	pub fn stop(&self) {
		self.shutdown.store(true, Ordering::SeqCst);
		self.wake.notify_waiters();
	}

	/// Stops the workers and waits for the jobs they hold to finish.
	pub async fn shutdown(self) {
		self.stop();

		for task in self.tasks {
			let _ = task.await;
		}
	}
}

/// Wakes idle workers when the jobs table changes on any instance: an
/// insert there is a job to run. The table is live, so the change arrives
/// over Postgres `NOTIFY`, or in process on SQLite.
async fn waker(db: Db, wake: Arc<Notify>, shutdown: Arc<AtomicBool>) {
	let Ok(mut feed) = db.system("wake job workers").changes().await else {
		// Without a feed the workers poll, which is slower but complete.
		return;
	};

	let table = match JobRow::NAMESPACE {
		Some(namespace) => format!("{namespace}.{}", JobRow::NAME),
		None => JobRow::NAME.to_owned(),
	};

	while !shutdown.load(Ordering::SeqCst) {
		let changed = tokio::select! {
			changed = feed.next() => changed,
			() = tokio::time::sleep(Duration::from_secs(1)) => continue,
		};

		let wakes = match changed {
			Ok(leptoskit_db::live::Invalidation::Rows(rows)) => {
				rows.iter().any(|rows| rows.table == table)
			}
			Ok(leptoskit_db::live::Invalidation::Everything) | Err(_) => true,
		};

		if wakes {
			wake.notify_waiters();
		}
	}
}

async fn worker(
	db: Db,
	settings: Settings,
	shutdown: Arc<AtomicBool>,
	wake: Arc<Notify>,
	poll: Duration,
	retention: Option<Duration>,
) {
	let mut pruned_at: Option<std::time::Instant> = None;

	while !shutdown.load(Ordering::SeqCst) {
		// Ask to be woken before looking, so an enqueue between the look
		// and the wait isn't missed.
		let woken = wake.notified();
		tokio::pin!(woken);
		woken.as_mut().enable();

		match run_one(&db, &settings, true).await {
			Ok(true) => continue,
			Ok(false) => {}
			Err(_) => {
				tokio::time::sleep(Duration::from_secs(1)).await;
				continue;
			}
		}

		let _ = sweep_failures(&db).await;

		if let Some(retention) = retention
			&& pruned_at.is_none_or(|at| at.elapsed() >= Duration::from_hours(1))
		{
			pruned_at = Some(std::time::Instant::now());
			let _ = prune(&db, retention).await;
		}

		if shutdown.load(Ordering::SeqCst) {
			break;
		}

		tokio::select! {
			() = woken => {}
			() = tokio::time::sleep(poll) => {}
		}
	}
}

/// Claims one due job and runs it. Whether one ran.
pub(crate) async fn run_one(
	db: &Db,
	settings: &Settings,
	heartbeat: bool,
) -> Result<bool, DbError> {
	let Some(row) = claim(db, settings).await? else {
		return Ok(false);
	};
	execute(db, settings, row, heartbeat).await;
	Ok(true)
}

/// The lease a job of `name` gets: the workers' own, or its timeout and a
/// margin, whichever is longer.
fn lease_for(settings: &Settings, name: &str) -> Duration {
	let timeout = job_registration(name).and_then(|registration| registration.options.timeout);
	timeout.map_or(settings.lease, |timeout| {
		settings.lease.max(timeout + Duration::from_secs(30))
	})
}

/// Takes the first due job this worker may run: waiting ones whose time has
/// come, and running ones whose worker's lease ran out.
async fn claim(db: &Db, settings: &Settings) -> Result<Option<JobRow>, DbError> {
	let system = db.system("claim jobs");
	let now = settings.clock.now();
	let candidates = JobRow::query()
		.filter(|job| {
			(job.state.eq(JobState::Pending) & job.run_at.lte(now))
				| (job.state.eq(JobState::Running) & job.locked_until.lt(now))
		})
		.order_by(|job| job.priority.desc())
		.order_by(|job| job.run_at.asc())
		.limit(32)
		.all(&system)
		.await?;

	for row in candidates {
		let lease = lease_for(settings, &row.name);
		let limit = settings.queues.get(&row.queue).copied();
		let taken = if limit.is_none() && row.serial_key.is_none() {
			take(&system, &row, now, lease).await?
		} else {
			match guarded_take(&system, &row, now, lease, limit).await {
				Ok(taken) => taken,
				// Another worker claimed in the same queue or series at the
				// same moment; one of the two wins, and this tries the next.
				Err(error) if error.is_contended() => false,
				Err(error) => return Err(error),
			}
		};

		if taken {
			let mut claimed = row;
			claimed.state = JobState::Running;
			claimed.attempts += 1;
			claimed.started_at = Some(now);
			claimed.locked_until = Some(now + duration(lease));

			return Ok(Some(claimed));
		}
	}

	Ok(None)
}

/// Marks `row` running for this worker, if nobody else did first: the
/// attempt count is the fence, so a worker that lost its lease can't record
/// over the one that took the job over.
async fn take(
	connection: &impl Connection,
	row: &JobRow,
	now: Timestamp,
	lease: Duration,
) -> Result<bool, DbError> {
	let (id, state, attempts) = (row.id, row.state, row.attempts);
	let changed = JobRow::update()
		.filter(|job| job.id.eq(id) & job.state.eq(state) & job.attempts.eq(attempts))
		.set(|job| job.state, JobState::Running)
		.set(|job| job.attempts, attempts + 1)
		.set(|job| job.started_at, now)
		.set(|job| job.locked_until, now + duration(lease))
		.run(connection)
		.await?;
	Ok(changed == 1)
}

/// Takes `row` only if its queue is under `limit` and no other job in its
/// series runs, in one serializable transaction: two workers that each see
/// room and each take a job conflict, and Postgres makes one of them fail,
/// so the limit holds across instances. SQLite has one writer, so it holds
/// there already.
async fn guarded_take(
	system: &Db,
	row: &JobRow,
	now: Timestamp,
	lease: Duration,
	limit: Option<usize>,
) -> Result<bool, DbError> {
	let tx = system.begin_serializable().await?;
	let id = row.id;

	let room = match limit {
		Some(limit) => {
			let queue = row.queue.clone();
			let running = JobRow::query()
				.filter(|job| {
					job.queue.eq(queue)
						& job.state.eq(JobState::Running)
						& job.locked_until.gte(now)
						& job.id.ne(id)
				})
				.count(&tx)
				.await?;
			usize::try_from(running).unwrap_or(usize::MAX) < limit
		}
		None => true,
	};

	let free = match (&row.serial_key, room) {
		(Some(serial), true) => {
			let serial = serial.clone();
			!JobRow::query()
				.filter(|job| {
					job.serial_key.eq(serial)
						& job.state.eq(JobState::Running)
						& job.locked_until.gte(now)
						& job.id.ne(id)
				})
				.exists(&tx)
				.await?
		}
		(_, room) => room,
	};

	if !free || !take(&tx, row, now, lease).await? {
		tx.rollback().await?;

		return Ok(false);
	}

	tx.commit().await?;
	Ok(true)
}

/// How one run ended.
enum Outcome {
	Done(serde_json::Value),
	/// Its worker died, as far as anyone can tell: leave the row leased.
	Abandoned,
	Cancelled(String),
	Failed {
		error: String,
		permanent: bool,
	},
}

/// Runs a claimed job and records how it went.
async fn execute(db: &Db, settings: &Settings, row: JobRow, heartbeat: bool) {
	let owner = owner_actor(&row.owner);
	let context = owner_context(owner);
	let system = db.with_context(context.clone()).system("run jobs");
	let identity = Identity::new(context).with_caller(format!("job:{}", row.id));
	let now = settings.clock.now();
	let state = Arc::new(RunState::new(
		row.steps.0.clone(),
		row.cancel_requested,
		Progress {
			text: row.progress.clone(),
			fraction: row.fraction,
			dirty: false,
		},
		settings.faults.clone(),
	));
	let registration = job_registration(&row.name);

	let outcome = if let Some(outcome) = skip_reason(&row, now) {
		outcome
	} else if let Some(registration) = registration {
		run_handler(
			registration,
			ClaimedJob {
				system: system.clone(),
				identity,
				row: &row,
				owner,
				settings,
				heartbeat,
				state: &state,
			},
		)
		.await
	} else {
		// Another instance, on a newer build, may know it.
		Outcome::Failed {
			error: format!("no job is named `{}`; is its crate linked?", row.name),
			permanent: false,
		}
	};

	let _ = record(&system, settings, &row, &state, outcome).await;
}

/// Why a claimed job doesn't run, if it doesn't: it was cancelled, it
/// passed its deadline, or it ran its last attempt.
fn skip_reason(row: &JobRow, now: Timestamp) -> Option<Outcome> {
	if row.cancel_requested {
		return Some(Outcome::Cancelled("the job was cancelled".to_owned()));
	}

	if row.deadline_at.is_some_and(|deadline| deadline < now) {
		return Some(Outcome::Failed {
			error: "the job passed its deadline".to_owned(),
			permanent: true,
		});
	}

	if row.attempts > row.max_attempts {
		return Some(Outcome::Failed {
			error: row
				.error
				.clone()
				.unwrap_or_else(|| "the worker running its last attempt stopped".to_owned()),
			permanent: true,
		});
	}

	None
}

/// Everything one claimed job's run needs.
struct ClaimedJob<'a> {
	system: Db,
	identity: Identity,
	row: &'a JobRow,
	owner: Option<Actor>,
	settings: &'a Settings,
	heartbeat: bool,
	state: &'a Arc<RunState>,
}

/// Runs a claimed job's handler, with its timeout and heartbeat, and says
/// how it went.
async fn run_handler(registration: &'static JobRegistration, job: ClaimedJob<'_>) -> Outcome {
	let cx = JobCx::for_run(
		job.system.clone(),
		job.identity,
		job.row.id,
		job.row.attempts,
		job.owner,
		Arc::clone(job.state),
	);

	let running = (registration.handler)(JobRun {
		cx,
		arguments: job.row.arguments.0.clone(),
	});
	let beat = job.heartbeat.then(|| {
		tokio::spawn(beat(
			job.system.clone(),
			job.row.id,
			job.row.attempts,
			lease_for(job.settings, &job.row.name),
			job.settings.clock.clone(),
			Arc::clone(job.state),
		))
	});

	let result = match registration.options.timeout {
		Some(timeout) => {
			tokio::time::timeout(timeout, running)
				.await
				.unwrap_or_else(|_| {
					Err(Error::retry(format!(
						"the job ran longer than its timeout of {timeout:?}"
					)))
				})
		}
		None => running.await,
	};

	if let Some(beat) = beat {
		beat.abort();
	}

	match result {
		_ if job.state.is_killed() => Outcome::Abandoned,
		Ok(value) => Outcome::Done(value),

		Err(error) if job.state.is_cancelled() => Outcome::Cancelled(error.message().to_owned()),
		Err(error) => {
			Outcome::Failed {
				permanent: error.is_permanent(),
				error: error.message().to_owned(),
			}
		}
	}
}

/// Keeps a running job's lease, writes its progress a few times a second,
/// and notices when someone cancels it.
async fn beat(
	db: Db,
	id: leptoskit_db::Id<JobRow>,
	attempt: u32,
	lease: Duration,
	clock: Clock,
	state: Arc<RunState>,
) {
	let renew_every = (lease / 3).max(PROGRESS_EVERY);
	let mut renewed = std::time::Instant::now();

	loop {
		tokio::time::sleep(PROGRESS_EVERY).await;
		let renew = renewed.elapsed() >= renew_every;
		let progress = state.take_progress();

		if !renew && progress.is_none() {
			continue;
		}

		let mut update = JobRow::update().filter(|job| {
			job.id.eq(id) & job.state.eq(JobState::Running) & job.attempts.eq(attempt)
		});

		if renew {
			renewed = std::time::Instant::now();
			update = update.set(|job| job.locked_until, clock.now() + duration(lease));
		}

		if let Some(progress) = progress {
			update = update
				.set(|job| job.progress, progress.text)
				.set(|job| job.fraction, progress.fraction);
		}

		let _ = update.run(&db).await;

		if renew
			&& let Ok(true) = JobRow::query()
				.filter(|job| job.id.eq(id) & job.cancel_requested.eq(true))
				.exists(&db)
				.await
		{
			state.cancel();
		}
	}
}

/// Writes how a run went, if this worker still holds the job.
async fn record(
	system: &Db,
	settings: &Settings,
	row: &JobRow,
	state: &RunState,
	outcome: Outcome,
) -> Result<(), DbError> {
	let now = settings.clock.now();
	let (id, attempt) = (row.id, row.attempts);
	let progress = state.progress();
	let update = JobRow::update()
		.filter(|job| job.id.eq(id) & job.state.eq(JobState::Running) & job.attempts.eq(attempt))
		.set(|job| job.locked_until, None::<Timestamp>)
		.set(|job| job.progress, progress.text)
		.set(|job| job.fraction, progress.fraction);

	let update = match outcome {
		Outcome::Abandoned => return Ok(()),
		Outcome::Done(value) => {
			update
				.set(|job| job.state, JobState::Done)
				.set(|job| job.result, Some(Json(value)))
				.set(|job| job.error, None::<String>)
				.set(|job| job.finished_at, now)
		}
		Outcome::Cancelled(error) => {
			update
				.set(|job| job.state, JobState::Cancelled)
				.set(|job| job.error, truncate(error))
				.set(|job| job.finished_at, now)
		}
		Outcome::Failed { error, permanent } => {
			let backoff = job_registration(&row.name)
				.map(|registration| registration.options.backoff)
				.unwrap_or_default();
			let random =
				std::collections::hash_map::RandomState::new().hash_one(row.id.to_string());
			let retry_at = now + duration(backoff.delay(row.attempts, random));
			let gives_up = permanent
				|| row.attempts >= row.max_attempts
				|| row.deadline_at.is_some_and(|deadline| retry_at > deadline);
			if gives_up {
				let on_failure = job_registration(&row.name)
					.is_some_and(|registration| registration.on_failure.is_some());
				update
					.set(|job| job.state, JobState::Failed)
					.set(|job| job.error, truncate(error))
					.set(|job| job.finished_at, now)
					.set(|job| job.failure_pending, on_failure)
			} else {
				update
					.set(|job| job.state, JobState::Pending)
					.set(|job| job.error, truncate(error))
					.set(|job| job.run_at, retry_at)
			}
		}
	};

	update.run(system).await.map(|_| ())
}

/// An error short enough for its column.
fn truncate(mut error: String) -> String {
	if error.len() > 2000 {
		let mut end = 2000;

		while !error.is_char_boundary(end) {
			end -= 1;
		}

		error.truncate(end);
	}

	error
}

/// Runs the `on_failure` mutation of jobs that gave up. Each runs once:
/// its receipt is keyed by the job, so two sweeps at once apply it once.
/// How many ran.
pub(crate) async fn sweep_failures(db: &Db) -> Result<usize, DbError> {
	let system = db.system("run jobs' on_failure");
	let failed = JobRow::query()
		.filter(|job| job.state.eq(JobState::Failed) & job.failure_pending.eq(true))
		.limit(16)
		.all(&system)
		.await?;
	let mut ran = 0;

	for row in failed {
		let id = row.id;
		let handler = job_registration(&row.name).and_then(|registration| registration.on_failure);

		let done = match handler {
			None => true,
			Some(handler) => {
				let owner = owner_actor(&row.owner);
				let context = owner_context(owner);
				let result = handler(FailureRun {
					db: db
						.with_context(context.clone())
						.system("run a job's on_failure"),
					identity: Identity::new(context).with_caller(format!("job:{id}")),
					id: id.to_string(),
					arguments: row.arguments.0,
					error: row.error.unwrap_or_default(),
					attempts: row.attempts,
				})
				.await;
				ran += 1;
				// A mutation that can't succeed, such as one whose arguments no
				// longer fit, isn't tried forever.
				result.is_ok()
					|| result.as_ref().is_err_and(|error| {
						error.is_permanent() || matches!(error, Error::Invalid(_))
					})
			}
		};

		if done {
			JobRow::update()
				.filter(|job| job.id.eq(id) & job.failure_pending.eq(true))
				.set(|job| job.failure_pending, false)
				.run(&system)
				.await?;
		}
	}

	Ok(ran)
}

/// Deletes finished and cancelled jobs that finished more than `age` ago,
/// freeing their keys. Failed jobs stay until someone retries them. How
/// many it deleted.
///
/// # Errors
///
/// When the database fails.
pub async fn prune(db: &Db, age: Duration) -> Result<u64, DbError> {
	prune_at(db, leptoskit_db::now(), age).await
}

pub(crate) async fn prune_at(db: &Db, now: Timestamp, age: Duration) -> Result<u64, DbError> {
	let cutoff = now
		.checked_sub_signed(duration(age))
		.unwrap_or(chrono::DateTime::<chrono::Utc>::MIN_UTC);
	JobRow::query()
		.filter(|job| {
			(job.state.eq(JobState::Done) | job.state.eq(JobState::Cancelled))
				& job.finished_at.lt(cutoff)
		})
		.delete(&db.system("prune finished jobs"))
		.await
}

/// The most ticks of one schedule enqueued at once, so a clock that jumped
/// far doesn't flood the queue.
const MOST_TICKS: usize = 1_000;

/// Enqueues each `#[rpc::cron]` job for every tick in `(after, until]`,
/// keyed by the tick, so however many instances do the same, each tick's
/// job is enqueued once. How many this call enqueued.
pub(crate) async fn enqueue_cron(
	db: &Db,
	after: Timestamp,
	until: Timestamp,
) -> crate::Result<usize> {
	let system = db.system("enqueue cron jobs");
	let mut enqueued = 0;

	for registration in inventory::iter::<CronRegistration>() {
		let mut from = after;

		for _ in 0..MOST_TICKS {
			let Some(tick) = registration.schedule.next_after(from) else {
				break;
			};

			if tick > until {
				break;
			}

			from = tick;
			let key = format!("cron:{}:{}", registration.name, tick.to_rfc3339());

			if (registration.enqueue)(system.clone(), key, tick).await? {
				enqueued += 1;
			}
		}
	}

	Ok(enqueued)
}

/// Enqueues cron jobs as their ticks come.
async fn scheduler(db: Db, clock: Clock, wake: Arc<Notify>, shutdown: Arc<AtomicBool>) {
	if inventory::iter::<CronRegistration>().next().is_none() {
		return;
	}

	let mut last = clock.now();

	while !shutdown.load(Ordering::SeqCst) {
		let next = inventory::iter::<CronRegistration>()
			.filter_map(|registration| registration.schedule.next_after(last))
			.min();
		let wait = next
			.and_then(|next| (next - clock.now()).to_std().ok())
			.unwrap_or_default()
			.min(Duration::from_secs(1));
		tokio::time::sleep(wait).await;
		let now = clock.now();

		if enqueue_cron(&db, last, now).await.is_ok() {
			last = now;
			wake.notify_waiters();
		}
	}
}
