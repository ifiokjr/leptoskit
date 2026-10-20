//! Clocks and timers that work in the browser, on the server, and in tests.
//!
//! Staleness, garbage collection, retry backoff, and interval refetching all
//! need two things the standard library cannot provide portably: a wall clock
//! on `wasm32-unknown-unknown`, where `SystemTime::now` panics, and a sleep
//! that neither requires a browser `window` nor a running tokio runtime.

use std::future::Future;
use std::pin::Pin;
use std::sync::OnceLock;
use std::time::Duration;

/// A boxed future that can be sent across threads.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

type SleepProvider = Box<dyn Fn(Duration) -> BoxFuture<'static, ()> + Send + Sync>;

static SLEEP: OnceLock<SleepProvider> = OnceLock::new();

/// The error returned by [`install_sleep`] when a timer was already installed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SleepAlreadyInstalled;

impl std::fmt::Display for SleepAlreadyInstalled {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.write_str("a sleep provider was already installed")
	}
}

impl std::error::Error for SleepAlreadyInstalled {}

/// Replaces the timer used for retry backoff, garbage collection, and
/// interval refetching.
///
/// The defaults cover the browser (`setTimeout`) and native targets (tokio
/// when a runtime is running, a helper thread otherwise). A host that is
/// neither — a wasm module embedded outside a browser, for instance —
/// installs its own timer here before creating a
/// [`QueryClient`](crate::QueryClient).
pub fn install_sleep(
	provider: impl Fn(Duration) -> BoxFuture<'static, ()> + Send + Sync + 'static,
) -> Result<(), SleepAlreadyInstalled> {
	SLEEP
		.set(Box::new(provider))
		.map_err(|_| SleepAlreadyInstalled)
}

/// Milliseconds since the Unix epoch.
///
/// Timestamps are wall-clock so that data fetched on the server carries a
/// meaningful age to the browser, and so persisted caches can judge staleness
/// after a reload.
#[must_use]
pub fn now_ms() -> u64 {
	#[cfg(target_arch = "wasm32")]
	{
		web_sys::js_sys::Date::now() as u64
	}
	#[cfg(not(target_arch = "wasm32"))]
	{
		std::time::SystemTime::now()
			.duration_since(std::time::UNIX_EPOCH)
			.map_or(0, |elapsed| elapsed.as_millis() as u64)
	}
}

/// Waits for `duration`.
pub fn sleep(duration: Duration) -> BoxFuture<'static, ()> {
	if let Some(provider) = SLEEP.get() {
		return provider(duration);
	}
	if duration.is_zero() {
		return Box::pin(std::future::ready(()));
	}
	Box::pin(default_sleep(duration))
}

#[cfg(not(target_arch = "wasm32"))]
async fn default_sleep(duration: Duration) {
	if tokio::runtime::Handle::try_current().is_ok() {
		tokio::time::sleep(duration).await;
	} else {
		// Without a runtime there is no timer driver, so a helper thread
		// sleeps and completes a oneshot channel instead.
		let (sender, receiver) = futures::channel::oneshot::channel::<()>();
		std::thread::spawn(move || {
			std::thread::sleep(duration);
			let _ = sender.send(());
		});
		let _ = receiver.await;
	}
}

#[cfg(target_arch = "wasm32")]
async fn default_sleep(duration: Duration) {
	if web_sys::window().is_none() {
		// Outside a browser window there is no `setTimeout`; the only safe
		// choice is to yield immediately so retries and intervals still make
		// progress instead of hanging forever.
		static WARNED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
		if !WARNED.swap(true, std::sync::atomic::Ordering::Relaxed) {
			leptos::logging::warn!(
				"leptoskit_query: no browser window and no sleep provider installed; timers \
				 resolve immediately. Call `leptoskit_query::install_sleep` in this host."
			);
		}
		return;
	}
	let (sender, receiver) = futures::channel::oneshot::channel::<()>();
	leptos::prelude::set_timeout(
		move || {
			let _ = sender.send(());
		},
		duration,
	);
	let _ = receiver.await;
}
