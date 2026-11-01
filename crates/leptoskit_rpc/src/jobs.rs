//! Background jobs as every target sees them: the [`Job`] trait the macro
//! implements, its options, and what a client reads about a job.

use std::time::Duration;

use serde::Deserialize;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

#[cfg(any(not(feature = "server"), feature = "postgres", feature = "sqlite"))]
use crate::Kind;
#[cfg(any(not(feature = "server"), feature = "postgres", feature = "sqlite"))]
use crate::Operation;

/// A job that runs in the background: the struct `#[rpc::job]` generates
/// from the function's arguments.
///
/// The struct exists on every target, so shared code can name it, but only
/// a server build enqueues and runs one.
pub trait Job: Serialize + DeserializeOwned + Clone + Send + Sync + 'static {
	/// The job's name.
	const NAME: &'static str;
	/// How it runs: its queue, retries, backoff, and limits, from the
	/// `#[rpc::job(...)]` options.
	const OPTIONS: JobOptions = JobOptions::DEFAULT;
	/// What a successful run returns.
	type Output: Serialize + DeserializeOwned + Send + Sync + 'static;

	/// Runs the body. The macro implements this.
	#[cfg(any(feature = "postgres", feature = "sqlite"))]
	fn run(
		self,
		cx: &crate::JobCx,
	) -> impl std::future::Future<Output = crate::Result<Self::Output>> + Send;
}

/// How a job runs, from `#[rpc::job(...)]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct JobOptions {
	/// Its queue: `queue = "mail"`. Each queue has its own concurrency.
	pub queue: &'static str,
	/// How many more times it runs after a failure: `retries = 8`. Four by
	/// default, so five attempts in all.
	pub retries: u32,
	/// How long it waits between attempts.
	pub backoff: Backoff,
	/// How long after it was enqueued it may still start an attempt:
	/// `deadline = 15m`. None by default.
	pub deadline: Option<Duration>,
	/// How long one attempt may run before it counts as failed: `timeout =
	/// 30s`. None by default.
	pub timeout: Option<Duration>,
	/// The argument whose value allows at most one running job at a time:
	/// `serialize_by = wallet`.
	pub serialize_by: Option<&'static str>,
	/// Who may read the job besides the person who enqueued it.
	pub visible_to: Visibility,
	/// Whether an `on_failure` mutation runs when it gives up.
	pub on_failure: bool,
}

impl JobOptions {
	/// The defaults: the `default` queue, four retries, a backoff from one
	/// second to an hour, no deadline, no timeout, and visible to its owner.
	pub const DEFAULT: Self = Self {
		queue: "default",
		retries: 4,
		backoff: Backoff::DEFAULT,
		deadline: None,
		timeout: None,
		serialize_by: None,
		visible_to: Visibility::Owner,
		on_failure: false,
	};
}

impl Default for JobOptions {
	fn default() -> Self {
		Self::DEFAULT
	}
}

/// How long a job waits before its next attempt: exponential, from `min`
/// doubling up to `max`, each wait a random part of the upper half of that,
/// so jobs that failed together don't all return together.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Backoff {
	/// The first wait.
	pub min: Duration,
	/// The longest wait.
	pub max: Duration,
}

impl Backoff {
	/// One second, doubling up to an hour.
	pub const DEFAULT: Self = Self::exponential(Duration::from_secs(1), Duration::from_secs(3_600));

	/// From `min`, doubling up to `max`: `backoff = exponential(2s, 5m)`.
	#[must_use]
	pub const fn exponential(min: Duration, max: Duration) -> Self {
		Self { min, max }
	}

	/// The longest wait before attempt `attempt + 1`, after `attempt`
	/// failures (the first is 1), before jitter.
	#[must_use]
	pub fn ceiling(&self, attempt: u32) -> Duration {
		let doublings = attempt.saturating_sub(1).min(32);
		self.min
			.saturating_mul(1_u32.checked_shl(doublings).unwrap_or(u32::MAX))
			.min(self.max)
			.max(self.min.min(self.max))
	}

	/// The wait before the next attempt after `attempt` failures: between
	/// half of [`ceiling`](Self::ceiling) and all of it, picked by `random`.
	#[must_use]
	pub fn delay(&self, attempt: u32, random: u64) -> Duration {
		let ceiling = u64::try_from(self.ceiling(attempt).as_millis()).unwrap_or(u64::MAX);
		let half = ceiling / 2;
		Duration::from_millis(half + random % (ceiling - half + 1))
	}
}

impl Default for Backoff {
	fn default() -> Self {
		Self::DEFAULT
	}
}

/// Who may read a job besides the person who enqueued it, from
/// `#[rpc::job(visible_to = ...)]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Visibility {
	/// Only its owner: the default.
	Owner,
	/// Anyone signed in: `visible_to = signed_in`.
	SignedIn,
	/// Anyone at all: `visible_to = everyone`.
	Everyone,
	/// Its owner, and the account an argument names: `visible_to =
	/// recipient`.
	Argument(&'static str),
}

/// What an `on_failure` mutation receives: the job's arguments, and why it
/// gave up.
///
/// ```ignore
/// #[rpc::mutation]
/// async fn payment_gave_up(cx: &rpc::MutationCx, job: rpc::Failed<SettlePayment>) -> rpc::Result<()> {
/// 	MarkPaymentFailed { payment: job.job.payment, reason: job.error }.run(cx).await
/// }
/// ```
#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(bound = "J: Serialize + DeserializeOwned")]
pub struct Failed<J> {
	/// The job's id.
	pub id: String,
	/// The arguments it was enqueued with.
	pub job: J,
	/// Why its last attempt failed.
	pub error: String,
	/// How many attempts it had.
	pub attempts: u32,
}

/// A job's state and result, as a client reads it.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize, schemars::JsonSchema)]
pub struct JobStatus {
	/// Where it is: `pending`, `running`, `done`, `failed`, or `cancelled`.
	/// A `pending` job with an [`error`](Self::error) is waiting to retry.
	pub state: String,
	/// Its queue.
	pub queue: String,
	/// How many times it started.
	pub attempts: u32,
	/// How many starts it gets.
	pub max_attempts: u32,
	/// Why the last attempt failed.
	pub error: Option<String>,
	/// What it returned, once done.
	pub result: Option<Value>,
	/// What it last said it was doing, through `cx.progress(...)`.
	pub progress: Option<String>,
	/// How far along it last said it was, from 0 to 1.
	pub fraction: Option<f64>,
	/// Whether someone asked it to stop.
	pub cancel_requested: bool,
	/// When it may run, as an RFC 3339 timestamp.
	pub run_at: String,
	/// When it finished, as an RFC 3339 timestamp.
	pub finished_at: Option<String>,
}

/// The arguments of the job-status query: the job's id.
#[derive(Clone, Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct JobStatusArgs {
	/// The job's id.
	pub id: String,
}

// A server build with no database driver has no `QueryCx` to name, so it
// has no job status to read; everything else does.
#[cfg(any(not(feature = "server"), feature = "postgres", feature = "sqlite"))]
impl Operation for JobStatusArgs {
	#[cfg(any(feature = "postgres", feature = "sqlite"))]
	type Context = crate::QueryCx;
	type Output = JobStatus;

	const KIND: Kind = Kind::Query;
	const NAME: &'static str = "rpc.job_status";

	#[cfg(any(feature = "postgres", feature = "sqlite"))]
	fn run(self, cx: &Self::Context) -> impl Future<Output = crate::Result<Self::Output>> + Send {
		crate::server::jobs::status(cx, self.id)
	}
}

/// Asks a job to stop: a waiting one never starts, and a running one sees
/// `cx.cancelled()` at its next step. Only the person who enqueued it may.
#[derive(Clone, Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct CancelJob {
	/// The job's id.
	pub id: String,
}

#[cfg(any(not(feature = "server"), feature = "postgres", feature = "sqlite"))]
impl Operation for CancelJob {
	#[cfg(any(feature = "postgres", feature = "sqlite"))]
	type Context = crate::MutationCx;
	type Output = ();

	const KIND: Kind = Kind::Mutation;
	const NAME: &'static str = "rpc.cancel_job";

	#[cfg(any(feature = "postgres", feature = "sqlite"))]
	async fn run(self, cx: &Self::Context) -> crate::Result<Self::Output> {
		let id = crate::server::jobs::parse_id(&self.id)?;
		crate::server::jobs::cancel_on(cx.tx(), id).await
	}
}

#[cfg(any(feature = "postgres", feature = "sqlite"))]
use std::future::Future;

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn backoff_doubles_up_to_its_ceiling_with_jitter() {
		let backoff = Backoff::exponential(Duration::from_secs(2), Duration::from_secs(300));
		assert_eq!(backoff.ceiling(1), Duration::from_secs(2));
		assert_eq!(backoff.ceiling(2), Duration::from_secs(4));
		assert_eq!(backoff.ceiling(8), Duration::from_secs(256));
		assert_eq!(backoff.ceiling(9), Duration::from_secs(300));
		assert_eq!(backoff.ceiling(200), Duration::from_secs(300));
		for random in [0, 1, 999, u64::MAX] {
			let wait = backoff.delay(3, random);
			assert!(wait >= Duration::from_secs(4), "{wait:?}");
			assert!(wait <= Duration::from_secs(8), "{wait:?}");
		}
		assert_eq!(Backoff::default(), Backoff::DEFAULT);
		assert_eq!(JobOptions::default(), JobOptions::DEFAULT);
	}
}
