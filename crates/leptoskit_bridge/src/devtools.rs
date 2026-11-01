//! The plugin calls timeline, for leptoskit's devtools.
//!
//! With the `devtools` feature, every [`Bridge`](crate::Bridge) records each
//! call as it moves through the transport: submitted by the guest, picked up
//! by the host, answered or cancelled, read by the guest, and answered late
//! after it was cancelled. Each change records the call's whole
//! [`CallRecord`] again, under the same id, so a reader keeps the latest
//! record for each id.
//!
//! Records go into one process-wide buffer of
//! [`CAPACITY`](leptoskit_devtools_protocol::sink::CAPACITY) records. A
//! devtools agent takes them with [`drain`]; when nothing drains in time, the
//! oldest are dropped and counted in [`dropped`]. Recording takes a lock only
//! to push, and does not allocate once the buffer exists.
//!
//! Times are milliseconds since a process-local epoch: the first time
//! anything in this module read the clock. An agent that wants times since
//! its session opened subtracts [`now_ms`] read when the session opened.

use std::sync::Arc;
#[cfg(not(target_arch = "wasm32"))]
use std::sync::OnceLock;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;

use leptoskit_devtools_protocol::CallRecord;
use leptoskit_devtools_protocol::CallStatus;
use leptoskit_devtools_protocol::sink::Sink;

use crate::CallId;
use crate::MethodId;

static CALLS: Sink<Trace> = Sink::new();
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// Takes every call change recorded since the last drain, oldest first.
pub fn drain() -> Vec<CallRecord> {
	CALLS.drain().into_iter().map(Trace::into_record).collect()
}

/// Calls `wake` after each recorded call change, so an agent that sleeps
/// between reports can send it soon after; `None` stops. It runs on the
/// thread that made the change.
pub fn set_waker(wake: Option<fn()>) {
	CALLS.set_waker(wake);
}

/// How many call changes were dropped because nothing drained them in time,
/// since the process started.
pub fn dropped() -> u64 {
	CALLS.dropped()
}

/// Milliseconds since this module's epoch, the clock every record uses.
///
/// Wasm has no monotonic clock without a host, and no bridge runs there, so
/// it reads zero on `wasm32`.
pub fn now_ms() -> u64 {
	#[cfg(not(target_arch = "wasm32"))]
	{
		static EPOCH: OnceLock<Instant> = OnceLock::new();
		let elapsed = EPOCH.get_or_init(Instant::now).elapsed();
		u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
	}
	#[cfg(target_arch = "wasm32")]
	{
		0
	}
}

/// Allocates the shared buffer up front, so the first recorded call does not.
pub(crate) fn reserve() {
	CALLS.reserve();
	let _ = now_ms();
}

/// One call's timeline, kept beside its bridge slot.
#[derive(Clone, Debug)]
pub(crate) struct Trace {
	pub(crate) call: CallId,
	id: u64,
	plugin: Arc<str>,
	method: MethodId,
	started_ms: u64,
	running_ms: Option<u64>,
	finished_ms: Option<u64>,
	taken_ms: Option<u64>,
	completed_late_ms: Option<u64>,
	status: CallStatus,
	request_bytes: u64,
	response_bytes: Option<u64>,
}

impl Trace {
	/// A call the guest just submitted, recorded.
	pub(crate) fn submitted(
		call: CallId,
		plugin: Arc<str>,
		method: MethodId,
		bytes: usize,
	) -> Self {
		let trace = Self {
			call,
			id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
			plugin,
			method,
			started_ms: now_ms(),
			running_ms: None,
			finished_ms: None,
			taken_ms: None,
			completed_late_ms: None,
			status: CallStatus::Pending,
			request_bytes: bytes as u64,
			response_bytes: None,
		};
		trace.record();
		trace
	}

	/// The host picked the call up.
	pub(crate) fn running(&mut self) {
		self.running_ms = Some(now_ms());
		self.record();
	}

	/// The host answered: with this many bytes, or with an error.
	pub(crate) fn answered(&mut self, bytes: Option<usize>) {
		self.finished_ms = Some(now_ms());

		self.status = match bytes {
			Some(_) => CallStatus::Ok,
			None => CallStatus::Error,
		};

		self.response_bytes = bytes.map(|bytes| bytes as u64);
		self.record();
	}

	/// A ready answer became an error, because its grant was revoked.
	#[cfg(all(feature = "async-host", not(target_arch = "wasm32")))]
	pub(crate) fn denied(&mut self) {
		self.status = CallStatus::Error;
		self.response_bytes = None;
		self.record();
	}

	/// The guest read the answer.
	pub(crate) fn taken(&mut self) {
		self.taken_ms = Some(now_ms());
		self.record();
	}

	/// The guest cancelled the call. A call that had already answered keeps
	/// the time it answered; its answer is dropped unread.
	pub(crate) fn cancelled(&mut self) {
		if self.finished_ms.is_none() {
			self.finished_ms = Some(now_ms());
		}

		self.status = CallStatus::Cancelled;
		self.response_bytes = None;
		self.record();
	}

	/// The host answered after the guest cancelled. Recorded once.
	pub(crate) fn completed_late(&mut self) {
		if self.completed_late_ms.is_none() {
			self.completed_late_ms = Some(now_ms());
			self.record();
		}
	}

	fn record(&self) {
		CALLS.record(self.clone());
	}

	fn into_record(self) -> CallRecord {
		CallRecord {
			id: self.id,
			plugin: self.plugin.as_ref().to_owned(),
			method: self.method.0.to_string(),
			started_ms: self.started_ms,
			running_ms: self.running_ms,
			finished_ms: self.finished_ms,
			taken_ms: self.taken_ms,
			completed_late_ms: self.completed_late_ms,
			status: self.status,
			request_bytes: Some(self.request_bytes),
			response_bytes: self.response_bytes,
		}
	}
}
