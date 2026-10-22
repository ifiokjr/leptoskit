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
/// when a runtime is running, a helper thread otherwise). A wasm module
/// hosted outside a browser has no default and must install one.
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
		.map_or(0, |elapsed| elapsed.as_millis() as u64)
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

// ---- embedded wasm ---------------------------------------------------------

#[cfg(all(target_arch = "wasm32", not(feature = "browser")))]
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
/// degrades to "fresh forever" with a clock stuck at zero. Said once.
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
