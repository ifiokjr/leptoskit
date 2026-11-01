//! What a job's body receives.

use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::PoisonError;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;

use leptoskit_db::Actor;
use leptoskit_db::Db;
use leptoskit_db::Id;
use leptoskit_db::Json;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use super::Enqueue;
use super::Faults;
use super::JobRow;
use super::JobState;
use crate::Error;
use crate::Job;
use crate::MutationCx;
use crate::Operation;
use crate::Result;
use crate::inbox::Notice;
use crate::server::Identity;
use crate::server::inbox::Recipient;

tokio::task_local! {
	/// The step running on this task, for keys and mutation receipts.
	static STEP: String;
}

/// What one run of a job knows as it goes: the steps already done, the
/// progress to write, and whether it was asked to stop.
#[derive(Default)]
pub(crate) struct RunState {
	steps: Mutex<serde_json::Map<String, Value>>,
	progress: Mutex<Progress>,
	cancelled: AtomicBool,
	killed: AtomicBool,
	faults: Option<Arc<Faults>>,
	mutations: Mutex<HashMap<String, u32>>,
}

/// What a job last said about how far along it is.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Progress {
	pub text: Option<String>,
	pub fraction: Option<f64>,
	/// Changed since it was last written.
	pub dirty: bool,
}

impl RunState {
	pub(crate) fn new(
		steps: Value,
		cancelled: bool,
		progress: Progress,
		faults: Option<Arc<Faults>>,
	) -> Self {
		Self {
			steps: Mutex::new(match steps {
				Value::Object(steps) => steps,
				_ => serde_json::Map::new(),
			}),
			progress: Mutex::new(progress),
			cancelled: AtomicBool::new(cancelled),
			killed: AtomicBool::new(false),
			faults,
			mutations: Mutex::default(),
		}
	}

	/// The progress to write, if it changed, marked written.
	pub(crate) fn take_progress(&self) -> Option<Progress> {
		let mut progress = self.progress.lock().unwrap_or_else(PoisonError::into_inner);
		if !progress.dirty {
			return None;
		}
		progress.dirty = false;
		Some(progress.clone())
	}

	/// The progress as it stands.
	pub(crate) fn progress(&self) -> Progress {
		self.progress
			.lock()
			.unwrap_or_else(PoisonError::into_inner)
			.clone()
	}

	pub(crate) fn cancel(&self) {
		self.cancelled.store(true, Ordering::Relaxed);
	}

	pub(crate) fn is_cancelled(&self) -> bool {
		self.cancelled.load(Ordering::Relaxed)
	}

	/// Whether a test's fault stopped the run, as if its worker died.
	pub(crate) fn is_killed(&self) -> bool {
		self.killed.load(Ordering::Relaxed)
	}
}

/// What a job's body receives: a database handle that bypasses policies,
/// who enqueued the job, and the tools to run it in steps.
pub struct JobCx {
	db: Db,
	identity: Identity,
	id: String,
	row: Option<Id<JobRow>>,
	attempt: u32,
	owner: Option<Actor>,
	state: Arc<RunState>,
}

impl crate::server::sealed::Sealed for JobCx {}

impl JobCx {
	/// A context for running a job's body directly, outside any worker, as
	/// `rpc::call(&db, SendInvite { … })` does: steps run every time, and
	/// nothing is saved.
	#[doc(hidden)]
	pub fn new(db: Db, identity: Identity, id: String, attempt: u32) -> Self {
		let owner = identity.context.actor();
		Self {
			db,
			identity,
			id,
			row: None,
			attempt,
			owner,
			state: Arc::default(),
		}
	}

	/// A context for one worker's run of the job `row`.
	pub(crate) fn for_run(
		db: Db,
		identity: Identity,
		row: Id<JobRow>,
		attempt: u32,
		owner: Option<Actor>,
		state: Arc<RunState>,
	) -> Self {
		Self {
			db,
			identity,
			id: row.to_string(),
			row: Some(row),
			attempt,
			owner,
			state,
		}
	}

	/// The database. It bypasses policies, since nobody's request is
	/// waiting, and records the job's owner as who made its changes.
	pub fn db(&self) -> &Db {
		&self.db
	}

	/// Who the job's mutations run as: its owner.
	pub fn identity(&self) -> &Identity {
		&self.identity
	}

	/// The job's id.
	pub fn id(&self) -> &str {
		&self.id
	}

	/// Which attempt this is: 1 the first time.
	pub fn attempt(&self) -> u32 {
		self.attempt
	}

	/// The account whose request enqueued the job, if someone signed in
	/// did: tell them how it went with [`notify`](Self::notify).
	pub fn owner(&self) -> Option<Actor> {
		self.owner
	}

	/// An idempotency key for an outside call: the same on every attempt of
	/// this job. Inside a [`step`](Self::step), it's the step's own key, so
	/// two steps calling the same service don't share one.
	///
	/// ```ignore
	/// cx.step("charge", async || stripe.charge(amount).idempotency_key(cx.key()).await).await?;
	/// ```
	pub fn key(&self) -> String {
		match STEP.try_with(Clone::clone) {
			Ok(step) => self.step_key(&step),
			Err(_) => self.id.clone(),
		}
	}

	/// The idempotency key of step `name`, wherever it's called from.
	pub fn step_key(&self, name: &str) -> String {
		format!("{}:{name}", self.id)
	}

	/// Whether someone asked the job to stop. A running job learns of it
	/// within a second or so, and at its next step at the latest, where
	/// [`step`](Self::step) fails for it.
	pub fn cancelled(&self) -> bool {
		self.state.is_cancelled()
	}

	/// Says what the job is doing, such as "Waiting for confirmation", for
	/// anyone watching it. Writes are spread out to a few a second, so
	/// call it as often as is useful.
	pub fn progress(&self, text: impl Into<String>) {
		let mut progress = self
			.state
			.progress
			.lock()
			.unwrap_or_else(PoisonError::into_inner);
		progress.text = Some(text.into());
		progress.dirty = true;
	}

	/// Says how far along the job is, from 0 to 1.
	pub fn progress_fraction(&self, fraction: f64) {
		let mut progress = self
			.state
			.progress
			.lock()
			.unwrap_or_else(PoisonError::into_inner);
		progress.fraction = Some(if fraction.is_nan() {
			0.0
		} else {
			fraction.clamp(0.0, 1.0)
		});
		progress.dirty = true;
	}

	/// Runs `work` once for this job: when it succeeds its result is saved
	/// with the job, and an attempt after a failure or a crash returns the
	/// saved result instead of running it again. A crash _during_ `work`
	/// runs it again, so work that reaches the outside world sends
	/// [`key`](Self::key).
	///
	/// ```ignore
	/// let receipt = cx.step("submit", async || chain::submit(&signed).await).await?;
	/// ```
	///
	/// Name each step once in a job: the name is how an attempt finds what
	/// the last one saved. Its result must serialize.
	///
	/// # Errors
	///
	/// What `work` fails with; [`Error::Permanent`] when the job was
	/// cancelled, which ends it; and [`Error::Retry`] when another worker
	/// took the job over meanwhile.
	pub async fn step<T, F, Fut>(&self, name: &str, work: F) -> Result<T>
	where
		T: Serialize + DeserializeOwned,
		F: FnOnce() -> Fut,
		Fut: Future<Output = Result<T>>,
	{
		let saved = self
			.state
			.steps
			.lock()
			.unwrap_or_else(PoisonError::into_inner)
			.get(name)
			.cloned();
		if let Some(saved) = saved {
			return serde_json::from_value(saved).map_err(|error| {
				Error::permanent(format!(
					"step `{name}` saved a result that no longer fits: {error}"
				))
			});
		}

		self.check_cancelled().await?;
		let value = STEP.scope(name.to_owned(), work()).await?;
		if self
			.state
			.faults
			.as_ref()
			.is_some_and(|faults| faults.kills(name))
		{
			// A test's fault: the worker dies before it saves the result.
			self.state.killed.store(true, Ordering::Relaxed);
			return Err(Error::server("the worker stopped"));
		}

		let saved = serde_json::to_value(&value).map_err(|error| {
			Error::permanent(format!("step `{name}`'s result isn't JSON: {error}"))
		})?;
		let steps = {
			let mut steps = self
				.state
				.steps
				.lock()
				.unwrap_or_else(PoisonError::into_inner);
			steps.insert(name.to_owned(), saved);
			Value::Object(steps.clone())
		};
		self.save(steps).await?;
		Ok(value)
	}

	/// Writes the steps done so far, and the progress, if this job is on
	/// its worker's lease still.
	async fn save(&self, steps: Value) -> Result<()> {
		let Some(row) = self.row else {
			return Ok(());
		};
		let attempt = self.attempt;
		let mut update = JobRow::update()
			.filter(|job| {
				job.id.eq(row) & job.state.eq(JobState::Running) & job.attempts.eq(attempt)
			})
			.set(|job| job.steps, Json(steps));
		if let Some(progress) = self.state.take_progress() {
			update = update
				.set(|job| job.progress, progress.text)
				.set(|job| job.fraction, progress.fraction);
		}
		if update.run(&self.db).await? == 0 {
			return Err(Error::retry("another worker took this job over"));
		}
		Ok(())
	}

	/// Fails when someone asked the job to stop.
	async fn check_cancelled(&self) -> Result<()> {
		if let Some(row) = self.row
			&& !self.state.is_cancelled()
			&& JobRow::query()
				.filter(|job| job.id.eq(row) & job.cancel_requested.eq(true))
				.exists(&self.db)
				.await?
		{
			self.state.cancel();
		}
		if self.state.is_cancelled() {
			return Err(Error::permanent("the job was cancelled"));
		}
		Ok(())
	}

	/// Runs a mutation once for this job: its receipt is keyed by the job,
	/// the step it's in, and how many times this step ran it before, so an
	/// attempt after a crash finds the result instead of applying it again.
	/// It runs as the job's owner, with policies bypassed.
	///
	/// ```ignore
	/// cx.mutate(MarkInviteSent { invite }).await?;
	/// ```
	///
	/// # Errors
	///
	/// What the mutation fails with, and [`Error::Conflict`] when this job
	/// ran the same mutation at the same point with other arguments before.
	pub async fn mutate<M: Operation<Context = MutationCx>>(&self, op: M) -> Result<M::Output> {
		let Some(_) = self.row else {
			return crate::server::run::mutate(&self.db, self.identity.clone(), None, op).await;
		};
		let scope = STEP.try_with(Clone::clone).unwrap_or_default();
		let count = {
			let mut mutations = self
				.state
				.mutations
				.lock()
				.unwrap_or_else(PoisonError::into_inner);
			let count = mutations.entry(format!("{scope}/{}", M::NAME)).or_default();
			*count += 1;
			*count
		};
		let request = format!("{scope}/{}/{count}", M::NAME);
		crate::server::run::mutate(&self.db, self.identity.clone(), Some(&request), op).await
	}

	/// Puts a notice in `account`'s inbox, such as the job's
	/// [`owner`](Self::owner), and hands it to the app's delivery channels.
	/// Inside a step it's sent once; outside one, every attempt that gets
	/// there sends it again, so notify at the end or from a step.
	///
	/// # Errors
	///
	/// When the notice can't be written.
	pub async fn notify(&self, account: impl Recipient, notice: Notice) -> Result<Option<String>> {
		let Some(delivery) = crate::server::inbox::insert(&self.db, &account, notice).await? else {
			return Ok(None);
		};
		let id = delivery.id.clone();
		tokio::spawn(crate::server::inbox::deliver(delivery));
		Ok(Some(id))
	}

	/// Enqueues another job, owned by this one's owner.
	pub fn enqueue<J: Job>(&self, job: J) -> Enqueue<'static, J> {
		Enqueue::on(self.db.clone(), job, self.owner)
	}
}

impl std::fmt::Debug for JobCx {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("JobCx")
			.field("id", &self.id)
			.field("attempt", &self.attempt)
			.finish_non_exhaustive()
	}
}
