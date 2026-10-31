//! Background jobs: work enqueued in a transaction, run later by a worker,
//! with retries, a lease, and a result the app can watch.
//!
//! ```ignore
//! #[rpc::mutation]
//! pub async fn invite(cx: &rpc::MutationCx, email: String) -> rpc::Result<()> {
//! 	let invite = NewInvite { email }.insert(cx.tx()).await?;
//! 	// The job exists exactly when the mutation commits.
//! 	cx.enqueue(SendInvite { invite: invite.id }).await?;
//! 	Ok(())
//! }
//!
//! #[rpc::job]
//! pub async fn send_invite(cx: &rpc::JobCx, invite: Id<Invite>) -> rpc::Result<()> {
//! 	// ...
//! 	Ok(())
//! }
//! ```
//!
//! Start the workers once, at boot:
//!
//! ```ignore
//! let jobs = rpc::server::Jobs::new(db.clone()).start();
//! ```
//!
//! A job runs at least once: a worker that dies after the outside service
//! accepted a request leaves the job leased, and the lease expiring runs it
//! again. Put operations with keys (an email service that accepts an
//! idempotency key) behind [`JobCx::id`], which is stable across attempts.

use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::time::Duration;

use leptoskit_db::Db;
use leptoskit_db::Enum;
use leptoskit_db::Expr;
use leptoskit_db::Id;
use leptoskit_db::Json;
use leptoskit_db::Timestamp;
use leptoskit_db::policy;
use leptoskit_db::table;
use leptoskit_query::BoxFuture;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use super::Identity;
use super::QueryCx;
use super::handle;
use crate::Error;
use crate::Operation;
use crate::Result;

/// Where a job is in its life.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Enum)]
pub enum JobState {
	/// Waiting for a worker, or for its `run_at`.
	Pending,
	/// A worker has it.
	Running,
	/// It finished.
	Done,
	/// It ran out of attempts, or failed without any.
	Failed,
	/// Nothing will run it.
	Cancelled,
}

/// One enqueued job.
#[table(name = "jobs", pack = "rpc", index(state, run_at), index(locked_until))]
#[policy(insert, |_, _| Expr::always())]
pub struct JobRow {
	pub id: Id<Self>,
	/// The app's idempotency key, if it gave one: enqueuing it again returns
	/// the same job.
	#[db(unique, max_len = 255)]
	pub key: Option<String>,
	/// The job operation's name.
	#[db(max_len = 255)]
	pub name: String,
	/// The operation's arguments.
	pub arguments: Json<Value>,
	/// Who enqueued it: their [`Identity::caller`](super::Identity::caller),
	/// or empty.
	#[db(max_len = 255)]
	pub owner: String,
	pub state: JobState,
	/// How many times a worker has started it.
	pub attempts: u32,
	/// How many starts it gets before it fails.
	pub max_attempts: u32,
	/// Higher runs first.
	pub priority: i32,
	/// Not before this instant.
	pub run_at: Timestamp,
	/// When a worker last started it.
	pub started_at: Option<Timestamp>,
	/// When it reached `done` or `failed`.
	pub finished_at: Option<Timestamp>,
	/// Until when the running worker holds it; `None` when no one does.
	pub locked_until: Option<Timestamp>,
	/// What the body returned, as JSON.
	pub result: Option<Json<Value>>,
	/// Why the last attempt failed.
	#[db(max_len = 2000)]
	pub error: Option<String>,
	#[db(created_at)]
	pub created_at: Timestamp,
}

/// What a job's body receives.
pub struct JobCx {
	db: Db,
	identity: Identity,
	id: String,
	attempt: u32,
}

impl super::sealed::Sealed for JobCx {}

impl JobCx {
	/// A context for one run. Public for the macro's expansion.
	#[doc(hidden)]
	pub fn new(db: Db, identity: Identity, id: String, attempt: u32) -> Self {
		Self {
			db,
			identity,
			id,
			attempt,
		}
	}

	/// The database, acting for the job.
	pub fn db(&self) -> &Db {
		&self.db
	}

	/// Who enqueued the job.
	pub fn identity(&self) -> &Identity {
		&self.identity
	}

	/// The job's id: stable across attempts, so outside services can use it
	/// as an idempotency key.
	pub fn id(&self) -> &str {
		&self.id
	}

	/// Which attempt this is: 1 the first time.
	pub fn attempt(&self) -> u32 {
		self.attempt
	}

	/// Enqueues another job when this one runs.
	pub fn enqueue<J: Job>(&self, job: J) -> Enqueue<J> {
		Enqueue::new(self.db.clone(), job)
	}
}

/// A job that runs in the background.
pub trait Job: Serialize + DeserializeOwned + Clone + Send + Sync + 'static {
	/// The job's name.
	const NAME: &'static str;
	/// What a successful run returns.
	type Output: Serialize + DeserializeOwned + Send + Sync + 'static;

	/// Runs the body. The macro implements this.
	#[cfg(feature = "server")]
	fn run(self, cx: &JobCx) -> impl Future<Output = Result<Self::Output>> + Send;
}

use std::future::Future;
use std::future::IntoFuture;

/// One run, as the worker hands it to the registered job.
pub struct JobRun {
	/// The database, acting for the job.
	pub db: Db,
	/// Who enqueued it.
	pub identity: Identity,
	/// The job's id.
	pub id: String,
	/// Which attempt this is.
	pub attempt: u32,
	/// The arguments, as JSON.
	pub arguments: Value,
}

/// What a registered job does.
pub type JobHandler = fn(JobRun) -> BoxFuture<'static, Result<Value>>;

/// One `#[rpc::job]`, as a server build registers it.
pub struct JobRegistration {
	/// The job's name.
	pub name: &'static str,
	/// Decodes the arguments, runs the body, and encodes the result.
	#[doc(hidden)]
	pub handler: JobHandler,
}

inventory::collect!(JobRegistration);

/// The job called `name`.
#[must_use]
pub fn job_registration(name: &str) -> Option<&'static JobRegistration> {
	inventory::iter::<JobRegistration>().find(|registration| registration.name == name)
}

/// Enqueues `job` on `db`, which should act as the caller or as the system.
///
/// ```ignore
/// rpc::enqueue(&db, SendInvite { invite }).after(Duration::from_secs(10)).await?;
/// ```
#[must_use]
pub fn enqueue<J: Job>(db: &Db, job: J) -> Enqueue<J> {
	Enqueue::new(db.clone(), job)
}

/// A job about to be enqueued, with when and how.
pub struct Enqueue<J: Job> {
	db: Db,
	job: J,
	after: Option<Duration>,
	at: Option<Timestamp>,
	key: Option<String>,
	priority: i32,
	max_attempts: Option<u32>,
}

impl<J: Job> Enqueue<J> {
	fn new(db: Db, job: J) -> Self {
		Self {
			db,
			job,
			after: None,
			at: None,
			key: None,
			priority: 0,
			max_attempts: None,
		}
	}

	/// Runs it no sooner than `after` from now.
	#[must_use]
	pub fn after(mut self, after: Duration) -> Self {
		self.after = Some(after);
		self
	}

	/// Runs it no sooner than `at`.
	#[must_use]
	pub fn at(mut self, at: Timestamp) -> Self {
		self.at = Some(at);
		self
	}

	/// An idempotency key: enqueuing the same key again returns the same
	/// job instead of adding another.
	#[must_use]
	pub fn key(mut self, key: impl Into<String>) -> Self {
		self.key = Some(key.into());
		self
	}

	/// Higher priorities run first.
	#[must_use]
	pub fn priority(mut self, priority: i32) -> Self {
		self.priority = priority;
		self
	}

	/// Gives it at most `attempts` starts.
	#[must_use]
	pub fn max_attempts(mut self, attempts: u32) -> Self {
		self.max_attempts = Some(attempts.max(1));
		self
	}

	async fn insert(self) -> Result<Id<JobRow>> {
		let owner = Identity::of(&self.db).caller.unwrap_or_default();
		insert(
			&self.db,
			&self.job,
			&owner,
			self.after,
			self.at,
			self.key,
			self.priority,
			self.max_attempts,
		)
		.await
	}
}

impl<J: Job> IntoFuture for Enqueue<J> {
	type IntoFuture = BoxFuture<'static, Result<Id<JobRow>>>;
	type Output = Result<Id<JobRow>>;

	fn into_future(self) -> Self::IntoFuture {
		Box::pin(self.insert())
	}
}

/// Inserts a job on `connection`, which may be a transaction, so the job
/// exists exactly when its caller commits.
#[expect(
	clippy::too_many_arguments,
	reason = "one enqueue's four settings plus its job, owner, and connection"
)]
pub(crate) async fn insert<J: Job>(
	connection: &impl leptoskit_db::Connection,
	job: &J,
	owner: &str,
	after: Option<Duration>,
	at: Option<Timestamp>,
	key: Option<String>,
	priority: i32,
	max_attempts: Option<u32>,
) -> Result<Id<JobRow>> {
	if let Some(key) = &key
		&& let Some(existing) = JobRow::query()
			.filter(|row| row.key.eq(key.clone()))
			.one(connection)
			.await
			.ok()
	{
		return Ok(existing.id);
	}

	let now = leptoskit_db::now();
	let run_at = at.unwrap_or_else(|| {
		now + chrono::Duration::from_std(after.unwrap_or_default())
			.unwrap_or_else(|_| chrono::Duration::zero())
	});
	let arguments = serde_json::to_value(job)
		.map_err(|error| Error::invalid(format!("a job's arguments aren't JSON: {error}")))?;
	let row = NewJobRow {
		key: key.clone(),
		name: J::NAME.to_owned(),
		arguments: Json(arguments),
		owner: owner.to_owned(),
		state: JobState::Pending,
		attempts: 0,
		max_attempts: max_attempts.unwrap_or(5),
		priority,
		run_at,
		started_at: None,
		finished_at: None,
		locked_until: None,
		result: None,
		error: None,
	}
	.insert(connection)
	.await;

	match row {
		Ok(row) => Ok(row.id),
		Err(leptoskit_db::DbError::Conflict { .. }) => {
			let Some(key) = key else {
				return Err(Error::conflict("that job already exists"));
			};
			JobRow::query()
				.filter(|row| row.key.eq(key))
				.one(connection)
				.await
				.map(|row| row.id)
				.map_err(Into::into)
		}
		Err(error) => Err(error.into()),
	}
}

/// The workers that run a database's jobs.
pub struct Jobs {
	db: Db,
	concurrency: usize,
	poll: Duration,
	lease: Duration,
}

impl Jobs {
	/// Workers for `db`'s jobs: four at a time, checking every 250 ms, with
	/// a one-minute lease each.
	pub fn new(db: Db) -> Self {
		Self {
			db,
			concurrency: 4,
			poll: Duration::from_millis(250),
			lease: Duration::from_secs(60),
		}
	}

	/// How many jobs run at once.
	#[must_use]
	pub fn concurrency(mut self, concurrency: usize) -> Self {
		self.concurrency = concurrency.max(1);
		self
	}

	/// How long a worker holds a job before another may take it over.
	#[must_use]
	pub fn lease(mut self, lease: Duration) -> Self {
		self.lease = lease;
		self
	}

	/// Starts the workers, which run until the returned handle stops them.
	pub fn start(self) -> JobsHandle {
		let shutdown = Arc::new(AtomicBool::new(false));
		let mut tasks = Vec::with_capacity(self.concurrency);
		for _ in 0..self.concurrency {
			let db = self.db.clone();
			let shutdown = Arc::clone(&shutdown);
			let poll = self.poll;
			let lease = self.lease;
			tasks.push(tokio::spawn(async move {
				worker(db, shutdown, poll, lease).await;
			}));
		}
		JobsHandle { shutdown, tasks }
	}
}

/// A running set of workers.
pub struct JobsHandle {
	shutdown: Arc<AtomicBool>,
	tasks: Vec<tokio::task::JoinHandle<()>>,
}

impl JobsHandle {
	/// Asks the workers to stop after their current job.
	pub fn stop(&self) {
		self.shutdown.store(true, Ordering::Relaxed);
	}

	/// Stops the workers and waits for them.
	pub async fn shutdown(self) {
		self.stop();
		for task in self.tasks {
			let _ = task.await;
		}
	}
}

async fn worker(db: Db, shutdown: Arc<AtomicBool>, poll: Duration, lease: Duration) {
	while !shutdown.load(Ordering::Relaxed) {
		match claim(&db, lease).await {
			Ok(Some(row)) => run(&db, row).await,
			Ok(None) => tokio::time::sleep(poll).await,
			Err(_) => tokio::time::sleep(Duration::from_secs(1)).await,
		}
	}
}

async fn claim(db: &Db, lease: Duration) -> leptoskit_db::Result<Option<JobRow>> {
	let system = db.system("claim jobs");
	let now = leptoskit_db::now();
	let candidates = JobRow::query()
		.filter(|job| job.state.eq(JobState::Pending) & job.run_at.lte(now))
		.order_by(|job| job.priority.desc())
		.order_by(|job| job.run_at.asc())
		.limit(16)
		.all(&system)
		.await?;

	for row in candidates {
		let lease =
			chrono::Duration::from_std(lease).unwrap_or_else(|_| chrono::Duration::seconds(60));
		let changed = JobRow::update()
			.filter(|job| job.id.eq(row.id) & job.state.eq(JobState::Pending))
			.set(|job| job.state, JobState::Running)
			.set(|job| job.attempts, row.attempts + 1)
			.set(|job| job.started_at, now)
			.set(|job| job.locked_until, now + lease)
			.run(&system)
			.await?;
		if changed == 1 {
			let mut claimed = row;
			claimed.state = JobState::Running;
			claimed.attempts += 1;
			claimed.started_at = Some(now);
			claimed.locked_until = Some(now + lease);
			return Ok(Some(claimed));
		}
	}
	Ok(None)
}

async fn run(db: &Db, row: JobRow) {
	let system = db.system("run jobs");
	let result = match job_registration(&row.name) {
		Some(registration) => {
			(registration.handler)(JobRun {
				db: system.clone(),
				identity: Identity::anonymous(),
				id: row.id.to_string(),
				attempt: row.attempts,
				arguments: row.arguments.0.clone(),
			})
			.await
		}
		None => {
			Err(Error::server(format!(
				"no job is named `{}`; is its crate linked?",
				row.name
			)))
		}
	};

	match result {
		Ok(value) => {
			let now = leptoskit_db::now();
			let _ = JobRow::update()
				.filter(|job| job.id.eq(row.id))
				.set(|job| job.state, JobState::Done)
				.set(|job| job.result, Some(Json(value)))
				.set(|job| job.finished_at, now)
				.set(|job| job.locked_until, None::<Timestamp>)
				.set(|job| job.error, None::<String>)
				.run(&system)
				.await;
		}
		Err(error) => {
			let now = leptoskit_db::now();
			let failed = row.attempts >= row.max_attempts;
			let retry_at = now
				+ chrono::Duration::seconds(2_i64.saturating_pow(row.attempts.min(10)).min(3_600));
			let update = JobRow::update()
				.filter(|job| job.id.eq(row.id))
				.set(
					|job| job.state,
					if failed {
						JobState::Failed
					} else {
						JobState::Pending
					},
				)
				.set(|job| job.error, error.message().to_owned())
				.set(|job| job.locked_until, None::<Timestamp>);
			let update = if failed {
				update.set(|job| job.finished_at, now)
			} else {
				update.set(|job| job.run_at, retry_at)
			};
			let _ = update.run(&system).await;
		}
	}
}

/// Reads a job's status for [`JobStatusArgs`](crate::jobs::JobStatusArgs).
pub async fn status(cx: &QueryCx, id: String) -> Result<crate::jobs::JobStatus> {
	let id: Id<JobRow> = id
		.parse()
		.map_err(|_| Error::invalid("that isn't a job id"))?;
	let system = cx.system("read a job's status");
	let row = JobRow::query()
		.filter(|job| job.id.eq(id))
		.one(&system)
		.await?;
	if row.owner != cx.identity().caller.clone().unwrap_or_default() {
		return Err(Error::not_found("no such job"));
	}
	Ok(crate::jobs::JobStatus {
		state: match row.state {
			JobState::Pending => "pending",
			JobState::Running => "running",
			JobState::Done => "done",
			JobState::Failed => "failed",
			JobState::Cancelled => "cancelled",
		}
		.to_owned(),
		attempts: row.attempts,
		max_attempts: row.max_attempts,
		error: row.error,
		result: row.result.map(|result| result.0),
		run_at: row.run_at.to_rfc3339(),
		finished_at: row.finished_at.map(|at| at.to_rfc3339()),
	})
}

#[allow(deprecated)]
const _: () = {
	inventory::submit! {
		super::Registration {
			name: <crate::jobs::JobStatusArgs as Operation>::NAME,
			kind: <crate::jobs::JobStatusArgs as Operation>::KIND,
			module_path: module_path!(),
			handler: handle::<crate::jobs::JobStatusArgs>,
		}
	}
};
