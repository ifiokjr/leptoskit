//! What a mutation leaves behind for after it commits: work to run, jobs to
//! enqueue, and ephemeral messages to publish.
//!
//! Everything here is recorded during the transaction so that a retried run
//! starts from nothing, and is released only once the transaction commits.
//! Jobs and channel messages are typed stubs until the job workers (L5) and
//! channels (L4) land: the mutation records them with the right shape, and
//! nothing consumes them yet.

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::PoisonError;
use std::time::Duration;

use leptoskit_db::Timestamp;
use leptoskit_query::BoxFuture;
use serde::Serialize;
use serde_json::Value;

/// One job a mutation enqueued, with the options `cx.enqueue(…).after(…)`,
/// `.at(…)`, `.key(…)`, and `.priority(…)` set.
#[derive(Clone, Debug, PartialEq)]
pub struct JobRecord {
	/// The job's type name, such as `my_app::SendInvite`.
	pub name: &'static str,
	/// Its arguments, as JSON.
	pub arguments: Value,
	/// How long after the commit it should run.
	pub after: Option<Duration>,
	/// The instant it should run.
	pub at: Option<Timestamp>,
	/// A key that makes enqueuing the same job twice safe.
	pub key: Option<String>,
	/// Lower values run first within a queue.
	pub priority: i32,
}

impl JobRecord {
	pub(crate) fn new<J: Serialize>(job: J) -> Self {
		Self {
			name: std::any::type_name::<J>(),
			arguments: serde_json::to_value(job).unwrap_or(Value::Null),
			after: None,
			at: None,
			key: None,
			priority: 0,
		}
	}
}

/// A job a mutation enqueued, and the options still to set on it.
///
/// It lives in the mutation's transaction: if the body fails or is retried,
/// nothing is enqueued. The job workers that run queued jobs arrive in L5;
/// until then the record is kept and released with the mutation's effects.
#[derive(Clone, Debug)]
#[must_use = "a job does nothing unless the mutation commits"]
pub struct Enqueued(Arc<Mutex<JobRecord>>);

impl Enqueued {
	pub(crate) fn new(record: JobRecord) -> Self {
		Self(Arc::new(Mutex::new(record)))
	}

	/// Runs the job no sooner than `delay` after the mutation commits.
	pub fn after(self, delay: Duration) -> Self {
		self.update(|record| record.after = Some(delay));
		self
	}

	/// Runs the job no sooner than `at`.
	pub fn at(self, at: Timestamp) -> Self {
		self.update(|record| record.at = Some(at));
		self
	}

	/// At most one queued or running job with this key, so enqueuing twice
	/// is safe.
	pub fn key(self, key: impl Into<String>) -> Self {
		let key = key.into();
		self.update(|record| record.key = Some(key));
		self
	}

	/// Runs before jobs with a higher priority in the same queue.
	pub fn priority(self, priority: i32) -> Self {
		self.update(|record| record.priority = priority);
		self
	}

	/// The job as it was recorded.
	#[must_use]
	pub fn record(&self) -> JobRecord {
		self.0
			.lock()
			.unwrap_or_else(PoisonError::into_inner)
			.clone()
	}

	fn update(&self, change: impl FnOnce(&mut JobRecord)) {
		change(&mut self.0.lock().unwrap_or_else(PoisonError::into_inner));
	}
}

/// One ephemeral message a mutation published, sent after it commits.
#[derive(Clone, Debug, PartialEq)]
pub struct Published {
	/// The channel it was published to: a channel's name until typed
	/// channels land in L4.
	pub channel: String,
	/// The message, as JSON.
	pub message: Value,
}

/// What a committed mutation releases.
pub(crate) struct Effects {
	pub(crate) after_commit: Vec<BoxFuture<'static, ()>>,
	pub(crate) jobs: Vec<Enqueued>,
	pub(crate) publications: Vec<Published>,
}

impl Effects {
	/// Drops the effects of a run that was retried, without running any.
	pub(crate) fn discard(self) {
		drop(self);
	}

	/// Runs what can run now: the after-commit work, in the order it was
	/// queued. Jobs and messages wait for the workers and channels that
	/// deliver them.
	pub(crate) fn release(self) {
		for work in self.after_commit {
			tokio::spawn(work);
		}

		drop(self.jobs);
		drop(self.publications);
	}
}
