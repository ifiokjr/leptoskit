//! What a client reads about a background job.

use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;

#[cfg(any(not(feature = "server"), feature = "postgres", feature = "sqlite"))]
use crate::Kind;
#[cfg(any(not(feature = "server"), feature = "postgres", feature = "sqlite"))]
use crate::Operation;

/// A job's state and result, as a client reads it.
#[derive(Clone, Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct JobStatus {
	/// Where it is: `pending`, `running`, `done`, `failed`, or `cancelled`.
	pub state: String,
	/// How many times it started.
	pub attempts: u32,
	/// How many starts it gets.
	pub max_attempts: u32,
	/// Why the last attempt failed.
	pub error: Option<String>,
	/// What it returned, once done.
	pub result: Option<Value>,
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

#[cfg(any(feature = "postgres", feature = "sqlite"))]
use std::future::Future;
