//! Clocks and timers for every host: browser, server, native, and embedded
//! wasm.
//!
//! Staleness, garbage collection, retry backoff, and interval refetching all
//! need a wall clock and a sleep. Neither comes for free everywhere:
//! `SystemTime::now` panics on `wasm32-unknown-unknown`, `setTimeout` needs a
//! browser window, and tokio timers need a running runtime. The defaults
//! cover each target this crate is built for; a host that is none of them
//! installs its own with [`install_clock`] and [`install_sleep`].

use std::future::Future;
use std::pin::Pin;
use std::sync::OnceLock;
use std::time::Duration;

/// A boxed future that can be sent across threads.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

type SleepProvider = Box<dyn Fn(Duration) -> BoxFuture<'static, ()> + Send + Sync>;
type ClockProvider = Box<dyn Fn() -> u64 + Send + Sync>;

static SLEEP: OnceLock<SleepProvider> = OnceLock::new();
static CLOCK: OnceLock<ClockProvider> = OnceLock::new();

/// The error returned by [`install_sleep`] and [`install_clock`] when a
/// provider was already installed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AlreadyInstalled;

impl std::fmt::Display for AlreadyInstalled {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.write_str("a provider was already installed")
	}
}

impl std::error::Error for AlreadyInstalled {}

/// Replaces the timer used for retry backoff, garbage collection, and
/// interval refetching. Install it before creating a
/// [`QueryClient`](crate::QueryClient).
///
/// The defaults cover the browser (`setTimeout`) and native targets (tokio
/// when a runtime is running, one shared timer thread otherwise). A wasm
/// module hosted outside a browser has no default and must install one.
pub fn install_sleep(
	sleep: impl Fn(Duration) -> BoxFuture<'static, ()> + Send + Sync + 'static,
) -> Result<(), AlreadyInstalled> {
	SLEEP.set(Box::new(sleep)).map_err(|_| AlreadyInstalled)
}

/// Replaces the wall clock, which must return milliseconds since the Unix
/// epoch. Install it before creating a [`QueryClient`](crate::QueryClient).
///
/// The defaults cover the browser (`Date.now()`) and native targets
/// (`SystemTime`). A wasm module hosted outside a browser has no default and
/// must install one.
pub fn install_clock(
	clock: impl Fn() -> u64 + Send + Sync + 'static,
) -> Result<(), AlreadyInstalled> {
	CLOCK.set(Box::new(clock)).map_err(|_| AlreadyInstalled)
}

/// Milliseconds since the Unix epoch.
///
/// Timestamps are wall-clock so that data fetched on the server carries a
/// meaningful age to the browser, and so persisted caches can judge staleness
/// after a reload.
#[must_use]
pub fn now_ms() -> u64 {
	match CLOCK.get() {
		Some(clock) => clock(),
		None => default_now_ms(),
	}
}

/// A duration in whole milliseconds, saturating at `u64::MAX`.
pub(crate) fn millis(duration: Duration) -> u64 {
	u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

/// Whether [`sleep`] really waits: a timer was installed, or this target has
/// a default one. Without one every sleep resolves at once.
pub(crate) fn has_timer() -> bool {
	SLEEP.get().is_some() || cfg!(any(not(target_arch = "wasm32"), feature = "browser"))
}

/// Waits for `duration`.
pub fn sleep(duration: Duration) -> BoxFuture<'static, ()> {
	if let Some(sleep) = SLEEP.get() {
		return sleep(duration);
	}
	if duration.is_zero() {
		return Box::pin(std::future::ready(()));
	}
	Box::pin(default_sleep(duration))
}

// ---- native ----------------------------------------------------------------

#[cfg(not(target_arch = "wasm32"))]
fn default_now_ms() -> u64 {
	std::time::SystemTime::now()
		.duration_since(std::time::UNIX_EPOCH)
		.map_or(0, millis)
}

#[cfg(not(target_arch = "wasm32"))]
async fn default_sleep(duration: Duration) {
	if tokio::runtime::Handle::try_current().is_ok() {
		tokio::time::sleep(duration).await;
	} else {
		// Without a runtime there is no timer driver.
		timer_thread::sleep(duration).await;
	}
}

/// One process-wide thread that wakes sleeps at their deadlines, for native
/// hosts without a tokio runtime: however many sleeps are waiting, or were
/// abandoned before their deadline, they cost one thread between them.
#[cfg(not(target_arch = "wasm32"))]
mod timer_thread {
	use std::cmp::Ordering;
	use std::cmp::Reverse;
	use std::collections::BinaryHeap;
	use std::sync::OnceLock;
	use std::sync::mpsc;
	use std::sync::mpsc::RecvTimeoutError;
	use std::time::Duration;
	use std::time::Instant;

	use futures::channel::oneshot;

	const RUNS_FOREVER: &str = "the timer thread runs for the life of the process";

	struct Deadline {
		at: Instant,
		wake: oneshot::Sender<()>,
	}

	impl PartialEq for Deadline {
		fn eq(&self, other: &Self) -> bool {
			self.at == other.at
		}
	}

	impl Eq for Deadline {}

	impl PartialOrd for Deadline {
		fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
			Some(self.cmp(other))
		}
	}

	impl Ord for Deadline {
		fn cmp(&self, other: &Self) -> Ordering {
			self.at.cmp(&other.at)
		}
	}

	pub(super) async fn sleep(duration: Duration) {
		let Some(at) = Instant::now().checked_add(duration) else {
			// Further off than the clock can count: never.
			return std::future::pending::<()>().await;
		};
		let (wake, woken) = oneshot::channel();
		timer().send(Deadline { at, wake }).expect(RUNS_FOREVER);
		// The thread drops a waker only after waking it, or once this
		// receiver is gone.
		woken.await.expect(RUNS_FOREVER);
	}

	fn timer() -> &'static mpsc::Sender<Deadline> {
		static TIMER: OnceLock<mpsc::Sender<Deadline>> = OnceLock::new();
		TIMER.get_or_init(|| {
			let (sender, receiver) = mpsc::channel();
			std::thread::Builder::new()
				.name("leptoskit_query timer".to_owned())
				.spawn(move || run(&receiver))
				.expect("spawning the timer thread");
			sender
		})
	}

	fn run(receiver: &mpsc::Receiver<Deadline>) {
		let mut pending = BinaryHeap::<Reverse<Deadline>>::new();
		// Abandoned sleeps are pruned whenever the heap doubles, so it stays
		// proportional to the sleeps still waiting.
		let mut prune_at = 64;
		loop {
			let message = match pending.peek() {
				None => receiver.recv().map_err(|_| RecvTimeoutError::Disconnected),
				Some(Reverse(next)) => {
					receiver.recv_timeout(next.at.saturating_duration_since(Instant::now()))
				}
			};
			match message {
				Ok(deadline) => {
					pending.push(Reverse(deadline));
					if pending.len() >= prune_at {
						pending.retain(|Reverse(deadline)| !deadline.wake.is_canceled());
						prune_at = (pending.len() * 2).max(64);
					}
				}
				Err(RecvTimeoutError::Timeout) => {}
				Err(RecvTimeoutError::Disconnected) => return,
			}
			let now = Instant::now();
			while pending
				.peek()
				.is_some_and(|Reverse(deadline)| deadline.at <= now)
			{
				if let Some(Reverse(deadline)) = pending.pop() {
					// A sleep dropped meanwhile no longer listens.
					let _ = deadline.wake.send(());
				}
			}
		}
	}
}

// ---- browser ---------------------------------------------------------------

#[cfg(all(target_arch = "wasm32", feature = "browser"))]
fn default_now_ms() -> u64 {
	web_sys::js_sys::Date::now() as u64
}

#[cfg(all(target_arch = "wasm32", feature = "browser"))]
#[wasm_bindgen::prelude::wasm_bindgen]
extern "C" {
	/// The global `setTimeout`, which windows and workers both have, unlike
	/// `window.setTimeout`.
	#[wasm_bindgen(js_name = setTimeout)]
	fn global_set_timeout(handler: &wasm_bindgen::JsValue, timeout: i32);
}

#[cfg(all(target_arch = "wasm32", feature = "browser"))]
async fn default_sleep(duration: Duration) {
	let (sender, receiver) = futures::channel::oneshot::channel::<()>();
	let handler = wasm_bindgen::closure::Closure::once_into_js(move || {
		let _ = sender.send(());
	});
	// Browsers fire longer timeouts at once, so clamp to their maximum.
	let timeout = i32::try_from(duration.as_millis()).unwrap_or(i32::MAX);
	global_set_timeout(&handler, timeout);
	let _ = receiver.await;
}

// ---- leptoskit's native runtime
// ----------------------------------------------

/// The time leptoskit's native Wasm runtime last reported. The runtime has no
/// clock import; instead it calls the guest's `leptoskit_clock` export with
/// the wall clock before every entry point, so the value is current for the
/// whole of each entry.
#[cfg(any(test, all(target_arch = "wasm32", feature = "guest")))]
mod host_clock {
	use std::sync::atomic::AtomicU64;
	use std::sync::atomic::Ordering;

	/// Zero until the host first reports.
	static REPORTED: AtomicU64 = AtomicU64::new(0);

	/// Records a report: milliseconds since the Unix epoch. A host's
	/// nonsense (zero or negative) is ignored.
	pub(super) fn report(unix_ms: i64) {
		if let Ok(unix_ms) = u64::try_from(unix_ms)
			&& unix_ms > 0
		{
			REPORTED.store(unix_ms, Ordering::Relaxed);
		}
	}

	/// The last report, if there has been one.
	#[cfg_attr(
		feature = "browser",
		allow(dead_code, reason = "with both features, the browser's own clock wins")
	)]
	pub(super) fn now_ms() -> Option<u64> {
		match REPORTED.load(Ordering::Relaxed) {
			0 => None,
			reported => Some(reported),
		}
	}
}

/// Called by leptoskit's native runtime with the wall clock, in milliseconds
/// since the Unix epoch, before each call into the guest. Runtimes that
/// predate it never call it, and the clock stays at zero.
#[cfg(all(target_arch = "wasm32", feature = "guest"))]
#[allow(unsafe_code, reason = "the host can only call an unmangled export")]
#[unsafe(no_mangle)]
pub extern "C" fn leptoskit_clock(unix_ms: i64) {
	host_clock::report(unix_ms);
}

#[cfg(all(target_arch = "wasm32", feature = "guest", not(feature = "browser")))]
fn default_now_ms() -> u64 {
	host_clock::now_ms().unwrap_or_else(|| {
		warn_missing_host("a clock");
		0
	})
}

// ---- embedded wasm ---------------------------------------------------------

#[cfg(all(
	target_arch = "wasm32",
	not(feature = "browser"),
	not(feature = "guest")
))]
fn default_now_ms() -> u64 {
	warn_missing_host("a clock");
	0
}

#[cfg(all(target_arch = "wasm32", not(feature = "browser")))]
fn default_sleep(_duration: Duration) -> std::future::Ready<()> {
	warn_missing_host("a timer");
	std::future::ready(())
}

/// Timers and clocks the host did not provide resolve to nothing rather than
/// hang: retries run without backoff, unobserved queries are collected at
/// once, interval refetching is off (see [`has_timer`]), and staleness
/// degrades to "fresh forever" with a clock stuck at zero. Said once per
/// process, for whichever is missing first.
#[cfg(all(target_arch = "wasm32", not(feature = "browser")))]
fn warn_missing_host(what: &str) {
	static WARNED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
	if !WARNED.swap(true, std::sync::atomic::Ordering::Relaxed) {
		leptos::logging::warn!(
			"leptoskit_query: this host has no {what}. Call `leptoskit_query::install_clock` and \
			 `leptoskit_query::install_sleep` before creating a QueryClient."
		);
	}
}

#[cfg(test)]
mod tests {
	use super::host_clock;

	#[test]
	fn the_host_clock_keeps_the_last_sensible_report() {
		assert_eq!(host_clock::now_ms(), None);

		host_clock::report(1_700_000_000_000);
		assert_eq!(host_clock::now_ms(), Some(1_700_000_000_000));

		host_clock::report(0);
		host_clock::report(-5);
		assert_eq!(host_clock::now_ms(), Some(1_700_000_000_000));

		// The wall clock may step back; the report is the time, not a maximum.
		host_clock::report(1_600_000_000_000);
		assert_eq!(host_clock::now_ms(), Some(1_600_000_000_000));
	}
}
