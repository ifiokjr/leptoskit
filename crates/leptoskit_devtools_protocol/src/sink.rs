//! A bounded buffer that instrumented crates record into and an agent drains.
//!
//! The plugin bridge and the rpc client record into a process-wide [`Sink`]
//! when their `devtools` feature is on. Recording holds a lock only long
//! enough to push one record, and never allocates past the bound: when the
//! sink is full, the oldest record is dropped and counted, so an agent that
//! stops draining costs a fixed amount of memory and never slows the app.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::sync::MutexGuard;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;

/// How many records a sink keeps before it drops the oldest, the same bound
/// as the agent's event queue.
pub const CAPACITY: usize = 256;

/// A bounded, process-wide buffer of records, usable as a `static`.
///
/// ```
/// use leptoskit_devtools_protocol::sink::Sink;
///
/// static CALLS: Sink<u32> = Sink::new();
///
/// CALLS.record(1);
/// CALLS.record(2);
/// assert_eq!(CALLS.drain(), vec![1, 2]);
/// assert_eq!(CALLS.drain(), Vec::<u32>::new());
/// ```
#[derive(Debug)]
pub struct Sink<T> {
	records: Mutex<VecDeque<T>>,
	dropped: AtomicU64,
}

impl<T> Default for Sink<T> {
	fn default() -> Self {
		Self::new()
	}
}

impl<T> Sink<T> {
	/// An empty sink.
	#[must_use]
	pub const fn new() -> Self {
		Self {
			records: Mutex::new(VecDeque::new()),
			dropped: AtomicU64::new(0),
		}
	}

	/// Records one value, dropping the oldest when the sink holds
	/// [`CAPACITY`] already.
	pub fn record(&self, value: T) {
		let mut records = self.lock();
		if records.len() >= CAPACITY {
			records.pop_front();
			self.dropped.fetch_add(1, Ordering::Relaxed);
		}
		records.push_back(value);
	}

	/// Allocates room for [`CAPACITY`] records now, so recording never
	/// allocates in the buffer later.
	pub fn reserve(&self) {
		let mut records = self.lock();
		let missing = CAPACITY.saturating_sub(records.len());
		records.reserve_exact(missing);
	}

	/// Takes every record, oldest first.
	pub fn drain(&self) -> Vec<T> {
		self.lock().drain(..).collect()
	}

	/// How many records were dropped because nothing drained the sink in
	/// time, since the process started.
	pub fn dropped(&self) -> u64 {
		self.dropped.load(Ordering::Relaxed)
	}

	fn lock(&self) -> MutexGuard<'_, VecDeque<T>> {
		// A panic while pushing leaves the queue intact, so keep using it.
		self.records
			.lock()
			.unwrap_or_else(std::sync::PoisonError::into_inner)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn a_full_sink_drops_the_oldest_and_counts_it() {
		let sink = Sink::new();
		for value in 0..CAPACITY + 3 {
			sink.record(value);
		}
		let records = sink.drain();
		assert_eq!(records.len(), CAPACITY);
		assert_eq!(records.first(), Some(&3));
		assert_eq!(records.last(), Some(&(CAPACITY + 2)));
		assert_eq!(sink.dropped(), 3);
		assert_eq!(sink.drain(), Vec::<usize>::new());
	}
}
