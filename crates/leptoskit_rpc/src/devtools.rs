//! The server calls timeline, for leptoskit's devtools.
//!
//! With the `devtools` feature, every attempt [`call_with_retry`],
//! [`query`], and [`mutation_with_retry`] make is recorded as an
//! [`RpcAttempt`]: once when it starts, and again under the same id when it
//! finishes, with the [`FnError::class`] it failed with. An attempt dropped
//! before it finished, such as a query nobody reads any more, finishes with
//! the class `cancelled`. A reader keeps the latest record for each id.
//!
//! Records go into one process-wide buffer of
//! [`CAPACITY`](leptoskit_devtools_protocol::sink::CAPACITY) records. A
//! devtools agent takes them with [`drain`]; when nothing drains in time, the
//! oldest are dropped and counted in [`dropped`].
//!
//! Times are milliseconds since a process-local epoch: the first time
//! anything in this module read the clock, which is
//! [`leptoskit_query::now_ms`], so it works wherever queries do. An agent
//! that wants times since its session opened subtracts [`now_ms`] read when
//! the session opened.
//!
//! The function's name is the type name of the future the app's fetch
//! returned, or of the fetch closure for [`call_with_retry`], trimmed to the
//! function that made it: `my_app::api::list_clips`.
//!
//! [`call_with_retry`]: crate::call_with_retry
//! [`query`]: crate::query
//! [`mutation_with_retry`]: crate::mutation_with_retry

use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;

use leptoskit_devtools_protocol::RpcAttempt;
use leptoskit_devtools_protocol::sink::Sink;

use crate::FnError;

static ATTEMPTS: Sink<RpcAttempt> = Sink::new();
static NEXT_ID: AtomicU64 = AtomicU64::new(1);
/// The clock's reading at the epoch, plus one; zero until the clock first
/// reads something other than zero.
static EPOCH: AtomicU64 = AtomicU64::new(0);

/// Takes every attempt change recorded since the last drain, oldest first.
pub fn drain() -> Vec<RpcAttempt> {
	ATTEMPTS.drain()
}

/// How many attempt changes were dropped because nothing drained them in
/// time, since the process started.
pub fn dropped() -> u64 {
	ATTEMPTS.dropped()
}

/// Milliseconds since this module's epoch, the clock every record uses.
///
/// A native guest's clock reads zero until its host first reports the time;
/// until then, so does this.
pub fn now_ms() -> u64 {
	let now = leptoskit_query::now_ms();
	if now == 0 {
		return 0;
	}
	let epoch = match EPOCH.compare_exchange(0, now + 1, Ordering::Relaxed, Ordering::Relaxed) {
		Ok(_) => now,
		Err(stored) => stored - 1,
	};
	now.saturating_sub(epoch)
}

/// The function a type name points at: `app::api::clips::{{closure}}`
/// becomes `app::api::clips`.
fn function_name(type_name: &'static str) -> &'static str {
	let mut name = type_name;
	while let Some(trimmed) = name.strip_suffix("::{{closure}}") {
		name = trimmed;
	}
	name
}

/// One attempt in flight. Finish it with [`Attempt::finish`]; dropping it
/// first records it as cancelled.
pub(crate) struct Attempt {
	record: Option<RpcAttempt>,
}

impl Attempt {
	/// Records the start of the one-based `attempt` of `function`, a type
	/// name.
	pub(crate) fn start(function: &'static str, attempt: u32) -> Self {
		let record = RpcAttempt {
			id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
			function: function_name(function).to_owned(),
			attempt,
			started_ms: now_ms(),
			finished_ms: None,
			error: None,
		};
		ATTEMPTS.record(record.clone());
		Self {
			record: Some(record),
		}
	}

	/// Records how the attempt ended.
	pub(crate) fn finish(mut self, error: Option<&FnError>) {
		if let Some(record) = self.record.take() {
			finish(record, error.map(FnError::class));
		}
	}
}

impl Drop for Attempt {
	fn drop(&mut self) {
		if let Some(record) = self.record.take() {
			finish(record, Some("cancelled"));
		}
	}
}

fn finish(mut record: RpcAttempt, error: Option<&str>) {
	record.finished_ms = Some(now_ms());
	record.error = error.map(str::to_owned);
	ATTEMPTS.record(record);
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn closures_trim_to_the_function_that_made_them() {
		assert_eq!(
			function_name("app::api::clips::{{closure}}::{{closure}}"),
			"app::api::clips"
		);
		assert_eq!(function_name("app::api::clips"), "app::api::clips");
	}
}
