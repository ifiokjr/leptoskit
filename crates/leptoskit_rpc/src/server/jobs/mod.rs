//! Background jobs: work enqueued in a transaction, run later by a worker,
//! with retries, checkpointed steps, and a state the app can watch.
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
//! #[rpc::job(queue = "mail", retries = 8)]
//! pub async fn send_invite(cx: &rpc::JobCx, invite: Id<Invite>) -> rpc::Result<()> {
//! 	let invite = Invite::get(cx.db(), invite).await?;
//! 	cx.step("send", async || mailer.send(&invite, cx.key()).await).await?;
//! 	cx.mutate(MarkInviteSent { invite: invite.id }).await?;
//! 	Ok(())
//! }
//! ```
//!
//! Start the workers once, at boot:
//!
//! ```ignore
//! let jobs = rpc::server::Jobs::new(db.clone()).workers(4).queue("mail", 2).start();
//! ```
//!
//! A job runs at least once. A worker that dies part-way leaves the job
//! leased, and when the lease runs out another worker runs it again, from
//! its first unfinished [step](JobCx::step). A step that reaches the outside
//! world sends [`JobCx::key`], which is the same on every attempt, so a
//! service that accepts idempotency keys does the work once.

pub mod cron;
mod cx;
mod worker;

use std::future::IntoFuture;
use std::time::Duration;

pub use cx::JobCx;
use leptoskit_db::Actor;
use leptoskit_db::Connection;
use leptoskit_db::Context;
use leptoskit_db::Db;
use leptoskit_db::Enum;
use leptoskit_db::Expr;
use leptoskit_db::Id;
use leptoskit_db::Json;
use leptoskit_db::Timestamp;
use leptoskit_db::Tx;
use leptoskit_db::policy;
use leptoskit_db::table;
use leptoskit_query::BoxFuture;
use serde_json::Value;
pub(crate) use worker::Clock;
pub(crate) use worker::Faults;
pub use worker::Jobs;
pub use worker::JobsHandle;
pub(crate) use worker::Settings;
pub(crate) use worker::enqueue_cron;
pub use worker::prune;
pub(crate) use worker::prune_at as worker_prune_at;
pub(crate) use worker::run_one;
pub(crate) use worker::sweep_failures;

use super::Identity;
use super::MutationCx;
use super::QueryCx;
use super::handle;
use crate::Error;
use crate::Failed;
pub use crate::Job;
use crate::JobOptions;
use crate::Operation;
use crate::Result;
use crate::Visibility;

/// Where a job is in its life.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Enum)]
pub enum JobState {
	/// Waiting for a worker, or for its `run_at`.
	Pending,
	/// A worker has it.
	Running,
	/// It finished.
	Done,
	/// It ran out of attempts, passed its deadline, or failed permanently.
	Failed,
	/// Someone cancelled it.
	Cancelled,
}

/// Who may read a job besides its owner, as stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Enum)]
pub enum JobVisibility {
	/// Its owner, and its [`viewer`](JobRow::viewer) if it has one.
	Owner,
	/// Anyone signed in.
	SignedIn,
	/// Anyone.
	Everyone,
}

/// One enqueued job, in the `rpc` pack's `jobs` table.
///
/// It's a live table: a client follows a job with
/// [`use_job`](crate::client::use_job), and the hub sends each change. Its
/// policies show a job to the account that enqueued it, and to whoever
/// `#[rpc::job(visible_to = ...)]` names, and let only the owner change it.
#[table(
	name = "jobs",
	pack = "rpc",
	live,
	index(state, run_at),
	index(locked_until),
	index(queue, state),
	index(serial_key, state),
	index(finished_at)
)]
#[policy(insert, |_, _| Expr::always())]
#[policy(select, |job, cx| job.visibility.eq(JobVisibility::Everyone) | job.owner
	.eq(cx.setting::<Actor>()) | job.viewer.eq(cx.setting::<Actor>()) | (job
	.visibility
	.eq(JobVisibility::SignedIn) & cx.setting::<Actor>().is_present()))]
#[policy(update, |job, cx| job.owner.eq(cx.setting::<Actor>()))]
pub struct JobRow {
	pub id: Id<Self>,
	/// The app's idempotency key, if it gave one: enqueuing it again returns
	/// the same job, until the job is pruned.
	#[db(unique, max_len = 255)]
	pub key: Option<String>,
	/// The job's name.
	#[db(max_len = 255)]
	pub name: String,
	/// Its queue.
	#[db(max_len = 255)]
	pub queue: String,
	/// The arguments, as JSON.
	pub arguments: Json<Value>,
	/// The account that enqueued it, as a UUID, or empty when nobody signed
	/// in did.
	#[db(max_len = 64)]
	pub owner: String,
	/// Another account that may read it, from `visible_to = argument`, or
	/// empty.
	#[db(max_len = 64)]
	pub viewer: String,
	/// Who else may read it.
	pub visibility: JobVisibility,
	/// The job's name and its `serialize_by` argument: at most one job with
	/// a key runs at a time.
	#[db(max_len = 512)]
	pub serial_key: Option<String>,
	pub state: JobState,
	/// How many times a worker has started it.
	pub attempts: u32,
	/// How many starts it gets before it fails.
	pub max_attempts: u32,
	/// Higher runs first.
	pub priority: i32,
	/// Not before this instant.
	pub run_at: Timestamp,
	/// No attempt starts after this instant.
	pub deadline_at: Option<Timestamp>,
	/// When a worker last started it.
	pub started_at: Option<Timestamp>,
	/// When it reached `done`, `failed`, or `cancelled`.
	pub finished_at: Option<Timestamp>,
	/// Until when the running worker holds it; `None` when no one does.
	pub locked_until: Option<Timestamp>,
	/// What the body returned, as JSON.
	pub result: Option<Json<Value>>,
	/// Why the last attempt failed.
	#[db(max_len = 2000)]
	pub error: Option<String>,
	/// What it last said it was doing.
	#[db(max_len = 500)]
	pub progress: Option<String>,
	/// How far along it last said it was, from 0 to 1.
	pub fraction: Option<f64>,
	/// The results of the steps it finished, by name.
	pub steps: Json<Value>,
	/// Someone asked it to stop.
	pub cancel_requested: bool,
	/// It failed, and its `on_failure` mutation hasn't run yet.
	pub failure_pending: bool,
	#[db(created_at)]
	pub created_at: Timestamp,
}

/// One run, as the worker hands it to the registered job.
pub struct JobRun {
	/// What the body receives.
	pub cx: JobCx,
	/// The arguments, as JSON.
	pub arguments: Value,
}

/// What a registered job does.
pub type JobHandler = fn(JobRun) -> BoxFuture<'static, Result<Value>>;

/// A job that gave up, as its `on_failure` mutation is run.
pub struct FailureRun {
	/// The database, acting as the system for the job's owner.
	pub db: Db,
	/// Who the mutation runs as.
	pub identity: Identity,
	/// The job's id.
	pub id: String,
	/// Its arguments.
	pub arguments: Value,
	/// Why its last attempt failed.
	pub error: String,
	/// How many attempts it had.
	pub attempts: u32,
}

/// What runs when a job gives up.
pub type FailureHandler = fn(FailureRun) -> BoxFuture<'static, Result<()>>;

/// One `#[rpc::job]`, as a server build registers it.
pub struct JobRegistration {
	/// The job's name.
	pub name: &'static str,
	/// How it runs.
	pub options: JobOptions,
	/// Decodes the arguments, runs the body, and encodes the result.
	#[doc(hidden)]
	pub handler: JobHandler,
	/// Runs its `on_failure` mutation.
	#[doc(hidden)]
	pub on_failure: Option<FailureHandler>,
}

inventory::collect!(JobRegistration);

/// The job called `name`.
#[must_use]
pub fn job_registration(name: &str) -> Option<&'static JobRegistration> {
	inventory::iter::<JobRegistration>().find(|registration| registration.name == name)
}

/// A mutation that can be a job's `on_failure`: it takes one argument,
/// `job: rpc::Failed<J>`.
#[diagnostic::on_unimplemented(
	message = "`{Self}` can't be the `on_failure` of `{J}`",
	label = "not a mutation taking `rpc::Failed<{J}>`",
	note = "an `on_failure` mutation takes one argument, `job: rpc::Failed<{J}>`, and is declared \
	        with #[rpc::mutation]"
)]
pub trait OnFailure<J>: Operation<Context = MutationCx> + From<Failed<J>> {}

impl<J, M: Operation<Context = MutationCx> + From<Failed<J>>> OnFailure<J> for M {}

/// Runs `M`, the `on_failure` of a job, once: its receipt is keyed by the
/// job, so a second sweep finds the first's result. Public for the macro's
/// expansion.
#[doc(hidden)]
pub async fn run_on_failure<M: OnFailure<J>, J>(
	db: Db,
	identity: Identity,
	failed: Failed<J>,
) -> Result<()> {
	super::run::mutate(&db, identity, Some("on_failure"), M::from(failed))
		.await
		.map(|_| ())
}

/// Checks a mutation can be `J`'s `on_failure`, where the macro names it.
#[doc(hidden)]
pub const fn assert_on_failure<M: OnFailure<J>, J>() {}

/// Enqueues `job` on `db`, owned by whoever `db` acts for.
///
/// ```ignore
/// rpc::enqueue(&db, SendInvite { invite }).after(Duration::from_secs(10)).await?;
/// ```
#[must_use]
pub fn enqueue<J: Job>(db: &Db, job: J) -> Enqueue<'static, J> {
	Enqueue::new(Place::Db(db.clone()), job, db.actor())
}

/// Where a job is inserted.
enum Place<'a> {
	Db(Db),
	Tx(&'a Tx),
}

/// A job about to be enqueued, with when and how. Await it to enqueue.
pub struct Enqueue<'a, J: Job> {
	place: Place<'a>,
	job: J,
	owner: Option<Actor>,
	settings: EnqueueSettings,
}

/// When and how one job is enqueued.
#[derive(Default)]
struct EnqueueSettings {
	after: Option<Duration>,
	at: Option<Timestamp>,
	key: Option<String>,
	priority: i32,
	max_attempts: Option<u32>,
	queue: Option<String>,
}

impl<'a, J: Job> Enqueue<'a, J> {
	fn new(place: Place<'a>, job: J, owner: Option<Actor>) -> Self {
		Self {
			place,
			job,
			owner,
			settings: EnqueueSettings::default(),
		}
	}

	/// In a mutation's transaction, owned by its caller.
	pub(crate) fn in_tx(tx: &'a Tx, job: J, owner: Option<Actor>) -> Self {
		Self::new(Place::Tx(tx), job, owner)
	}

	/// On a handle, owned by `owner`.
	pub(crate) fn on(db: Db, job: J, owner: Option<Actor>) -> Self {
		Self::new(Place::Db(db), job, owner)
	}

	/// Runs it no sooner than `after` from now.
	#[must_use]
	pub fn after(mut self, after: Duration) -> Self {
		self.settings.after = Some(after);
		self
	}

	/// Runs it no sooner than `at`.
	#[must_use]
	pub fn at(mut self, at: Timestamp) -> Self {
		self.settings.at = Some(at);
		self
	}

	/// An idempotency key: enqueuing the same key again returns the job
	/// already there instead of adding another.
	#[must_use]
	#[expect(
		clippy::needless_pass_by_value,
		reason = "a key is written `.key(payment.id)`: any value that displays"
	)]
	pub fn key(mut self, key: impl ToString) -> Self {
		self.settings.key = Some(key.to_string());
		self
	}

	/// Higher priorities run first within a queue.
	#[must_use]
	pub fn priority(mut self, priority: i32) -> Self {
		self.settings.priority = priority;
		self
	}

	/// Gives it at most `attempts` starts, instead of its `retries` plus
	/// one.
	#[must_use]
	pub fn max_attempts(mut self, attempts: u32) -> Self {
		self.settings.max_attempts = Some(attempts.max(1));
		self
	}

	/// Puts it in `queue` instead of its own.
	#[must_use]
	pub fn queue(mut self, queue: impl Into<String>) -> Self {
		self.settings.queue = Some(queue.into());
		self
	}

	async fn insert(self) -> Result<Id<JobRow>> {
		match &self.place {
			Place::Db(db) => insert(db, &self.job, self.owner, self.settings, false).await,
			Place::Tx(tx) => insert(*tx, &self.job, self.owner, self.settings, true).await,
		}
	}
}

impl<'a, J: Job> IntoFuture for Enqueue<'a, J> {
	type IntoFuture = BoxFuture<'a, Result<Id<JobRow>>>;
	type Output = Result<Id<JobRow>>;

	fn into_future(self) -> Self::IntoFuture {
		Box::pin(self.insert())
	}
}

/// An argument's value as text: a string as itself, anything else as JSON.
fn argument_text(arguments: &Value, field: &str) -> Option<String> {
	match arguments.get(field)? {
		Value::Null => None,
		Value::String(text) => Some(text.clone()),
		other => Some(other.to_string()),
	}
}

/// The owner column for `actor`.
fn owner_text(actor: Option<Actor>) -> String {
	actor
		.map(|actor| actor.uuid().to_string())
		.unwrap_or_default()
}

/// The actor an owner column names.
pub(crate) fn owner_actor(owner: &str) -> Option<Actor> {
	owner.parse::<Id<()>>().ok().map(Actor::of)
}

/// The context a job's work runs in: its owner's, so audit records say who
/// started it, though the work itself bypasses policies.
pub(crate) fn owner_context(owner: Option<Actor>) -> Context {
	owner.map_or_else(Context::new, |actor| Context::new().with(actor))
}

/// Inserts a job on `connection`, which may be a transaction, so the job
/// exists exactly when its caller commits. In a transaction, a key taken by
/// a job another transaction inserted meanwhile fails as contended, so the
/// mutation runs again and finds it.
async fn insert<J: Job>(
	connection: &impl Connection,
	job: &J,
	owner: Option<Actor>,
	settings: EnqueueSettings,
	in_transaction: bool,
) -> Result<Id<JobRow>> {
	if let Some(key) = &settings.key
		&& let Some(existing) = JobRow::query()
			.filter(|row| row.key.eq(key.clone()))
			.first(connection)
			.await?
	{
		return Ok(existing.id);
	}

	let options = J::OPTIONS;
	let now = leptoskit_db::now();
	let run_at = settings
		.at
		.unwrap_or_else(|| now + duration(settings.after.unwrap_or_default()));
	let arguments = serde_json::to_value(job)
		.map_err(|error| Error::invalid(format!("a job's arguments aren't JSON: {error}")))?;
	let serial_key = options.serialize_by.map(|field| {
		format!(
			"{}:{}",
			J::NAME,
			argument_text(&arguments, field).unwrap_or_default()
		)
	});
	let (visibility, viewer) = match options.visible_to {
		Visibility::Owner => (JobVisibility::Owner, String::new()),
		Visibility::SignedIn => (JobVisibility::SignedIn, String::new()),
		Visibility::Everyone => (JobVisibility::Everyone, String::new()),
		Visibility::Argument(field) => {
			(
				JobVisibility::Owner,
				argument_text(&arguments, field).unwrap_or_default(),
			)
		}
	};
	let row = NewJobRow {
		key: settings.key.clone(),
		name: J::NAME.to_owned(),
		queue: settings.queue.unwrap_or_else(|| options.queue.to_owned()),
		arguments: Json(arguments),
		owner: owner_text(owner),
		viewer,
		visibility,
		serial_key,
		state: JobState::Pending,
		attempts: 0,
		max_attempts: settings
			.max_attempts
			.unwrap_or_else(|| options.retries.saturating_add(1)),
		priority: settings.priority,
		run_at,
		deadline_at: options.deadline.map(|deadline| now + duration(deadline)),
		started_at: None,
		finished_at: None,
		locked_until: None,
		result: None,
		error: None,
		progress: None,
		fraction: None,
		steps: Json(Value::Object(serde_json::Map::new())),
		cancel_requested: false,
		failure_pending: false,
	}
	.insert(connection)
	.await;

	match row {
		Ok(row) => Ok(row.id),
		Err(leptoskit_db::DbError::Conflict { .. }) if settings.key.is_some() => {
			if in_transaction {
				return Err(Error::Contended);
			}
			let key = settings.key.unwrap_or_default();
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

/// `duration` for timestamps, saturating.
pub(crate) fn duration(duration: Duration) -> chrono::Duration {
	chrono::Duration::from_std(duration).unwrap_or(chrono::Duration::MAX)
}

/// A job id from text.
pub(crate) fn parse_id(id: &str) -> Result<Id<JobRow>> {
	id.parse()
		.map_err(|_| Error::invalid("that isn't a job id"))
}

/// Reads a job's status for [`JobStatusArgs`](crate::jobs::JobStatusArgs),
/// through the caller's policies: a job they may not see isn't there.
pub async fn status(cx: &QueryCx, id: String) -> Result<crate::jobs::JobStatus> {
	let id = parse_id(&id)?;
	let row = JobRow::query()
		.filter(|job| job.id.eq(id))
		.first(cx.db())
		.await?
		.ok_or_else(|| Error::not_found("no such job"))?;
	Ok(status_of(row))
}

/// What a client reads about `row`.
pub(crate) fn status_of(row: JobRow) -> crate::jobs::JobStatus {
	crate::jobs::JobStatus {
		state: row.state.as_str().to_owned(),
		queue: row.queue,
		attempts: row.attempts,
		max_attempts: row.max_attempts,
		error: row.error,
		result: row.result.map(|result| result.0),
		progress: row.progress,
		fraction: row.fraction,
		cancel_requested: row.cancel_requested,
		run_at: row.run_at.to_rfc3339(),
		finished_at: row.finished_at.map(|at| at.to_rfc3339()),
	}
}

/// Asks job `id` to stop, on `connection` as whoever it acts for: only the
/// job's owner may, or a handle that bypasses policies. A waiting job is
/// cancelled at once; a running one sees [`JobCx::cancelled`] at its next
/// step. A finished job is left as it is.
///
/// # Errors
///
/// [`Error::NotFound`] when the caller can't see the job, and
/// [`Error::Forbidden`] when they can but didn't enqueue it.
pub async fn cancel_on(connection: &impl Connection, id: Id<JobRow>) -> Result<()> {
	let row = JobRow::query()
		.filter(|job| job.id.eq(id))
		.first(connection)
		.await?
		.ok_or_else(|| Error::not_found("no such job"))?;
	let changed = match row.state {
		JobState::Pending => {
			JobRow::update()
				.filter(|job| job.id.eq(id) & job.state.eq(JobState::Pending))
				.set(|job| job.state, JobState::Cancelled)
				.set(|job| job.cancel_requested, true)
				.set(|job| job.finished_at, leptoskit_db::now())
				.run(connection)
				.await?
		}
		JobState::Running => {
			JobRow::update()
				.filter(|job| job.id.eq(id))
				.set(|job| job.cancel_requested, true)
				.run(connection)
				.await?
		}
		JobState::Done | JobState::Failed | JobState::Cancelled => return Ok(()),
	};
	if changed == 1 {
		return Ok(());
	}
	// Either the caller may read the job but not change it, or it moved on
	// meanwhile, such as a worker starting it: try again with what it is
	// now, which ends once it stops moving.
	let now = JobRow::query()
		.filter(|job| job.id.eq(id))
		.first(connection)
		.await?
		.ok_or_else(|| Error::not_found("no such job"))?;
	if now.state == row.state {
		return Err(Error::forbidden("only whoever started a job may cancel it"));
	}
	Box::pin(cancel_on(connection, id)).await
}

/// Asks job `id` to stop, as whoever `db` acts for. See [`cancel_on`].
///
/// # Errors
///
/// As [`cancel_on`].
pub async fn cancel(db: &Db, id: Id<JobRow>) -> Result<()> {
	cancel_on(db, id).await
}

/// Runs a failed or cancelled job again from the start, now, with its
/// attempts reset; its deadline no longer applies. `db` should bypass
/// policies: this is for operators.
///
/// # Errors
///
/// [`Error::NotFound`] when there's no such job, and [`Error::Conflict`]
/// when it's waiting, running, or done.
pub async fn retry(db: &Db, id: Id<JobRow>) -> Result<()> {
	let changed = JobRow::update()
		.filter(|job| {
			job.id.eq(id) & (job.state.eq(JobState::Failed) | job.state.eq(JobState::Cancelled))
		})
		.set(|job| job.state, JobState::Pending)
		.set(|job| job.attempts, 0_u32)
		.set(|job| job.run_at, leptoskit_db::now())
		.set(|job| job.deadline_at, None::<Timestamp>)
		.set(|job| job.finished_at, None::<Timestamp>)
		.set(|job| job.locked_until, None::<Timestamp>)
		.set(|job| job.cancel_requested, false)
		.set(|job| job.failure_pending, false)
		.run(db)
		.await?;
	if changed == 1 {
		return Ok(());
	}
	match JobRow::query()
		.filter(|job| job.id.eq(id))
		.first(db)
		.await?
	{
		None => Err(Error::not_found("no such job")),
		Some(row) => {
			Err(Error::conflict(format!(
				"only failed or cancelled jobs can be retried; this one is {}",
				row.state.as_str()
			)))
		}
	}
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
	inventory::submit! {
		super::Registration {
			name: <crate::jobs::CancelJob as Operation>::NAME,
			kind: <crate::jobs::CancelJob as Operation>::KIND,
			module_path: module_path!(),
			handler: handle::<crate::jobs::CancelJob>,
		}
	}
};

/// Runs a job's handler: decodes `arguments` as `J`, runs the body, and
/// encodes what it returns. Public for the macro's expansion.
#[doc(hidden)]
pub fn run_job<J: Job>(run: JobRun) -> BoxFuture<'static, Result<Value>> {
	Box::pin(async move {
		let job: J = serde_json::from_value(run.arguments).map_err(|error| {
			Error::permanent(format!("the arguments of `{}` don't fit: {error}", J::NAME))
		})?;
		let output = job.run(&run.cx).await?;
		serde_json::to_value(output).map_err(|_| Error::permanent("a job's result isn't JSON"))
	})
}

/// Runs `M`, the `on_failure` of job `J`, for a failure run. Public for the
/// macro's expansion.
#[doc(hidden)]
pub fn fail_job<J: Job, M: OnFailure<J>>(run: FailureRun) -> BoxFuture<'static, Result<()>> {
	Box::pin(async move {
		let job: J = serde_json::from_value(run.arguments).map_err(|error| {
			Error::permanent(format!("the arguments of `{}` don't fit: {error}", J::NAME))
		})?;
		let failed = Failed {
			id: run.id,
			job,
			error: run.error,
			attempts: run.attempts,
		};
		run_on_failure::<M, J>(run.db, run.identity, failed).await
	})
}
