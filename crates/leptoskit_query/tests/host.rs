//! An embedded host: no browser, no tokio — a clock and a timer the host
//! installs itself.

#![cfg(not(target_arch = "wasm32"))]

use std::sync::Arc;
use std::sync::atomic::AtomicU32;
use std::sync::atomic::Ordering;
use std::time::Duration;

mod common;

use common::Boom;
use common::Counter;
use common::settle;
use common::wait_for;
use leptos::prelude::*;
use leptoskit_query::*;

/// What an embedded host provides: a clock and a timer.
#[derive(Clone, Default)]
struct HostClock {
	ticks: Arc<std::sync::Mutex<Vec<Duration>>>,
	now: Arc<AtomicU32>,
}

impl HostClock {
	/// Installs this host's clock and timer the first time; every test in
	/// this binary then shares one host, as a real process would.
	fn shared() -> Self {
		static HOST: std::sync::OnceLock<HostClock> = std::sync::OnceLock::new();
		HOST.get_or_init(|| {
			let host = HostClock::default();
			let now = Arc::clone(&host.now);
			install_clock(move || u64::from(now.load(Ordering::SeqCst)) * 1_000)
				.expect("installed once");
			let ticks = Arc::clone(&host.ticks);
			install_sleep(move |duration| {
				let ticks = Arc::clone(&ticks);
				Box::pin(async move {
					ticks.lock().unwrap().push(duration);
				})
			})
			.expect("installed once");
			host
		})
		.clone()
	}

	fn advance(&self, seconds: u32) {
		self.now.fetch_add(seconds, Ordering::SeqCst);
	}

	fn waited_for(&self) -> Vec<Duration> {
		self.ticks.lock().unwrap().clone()
	}
}

#[tokio::test]
async fn the_hosts_clock_decides_staleness() {
	// The executor is the first thing any host provides.
	common::init();
	let host = HostClock::shared();
	let client = QueryClient::new();
	let calls = Counter::new();
	let scope: QueryScope<(), &'static str, Boom> = {
		let calls = calls.clone();
		QueryScope::new(move |()| {
			let calls = calls.clone();
			async move {
				calls.hit();
				Ok("data")
			}
		})
	}
	.with_name("host-staleness")
	.with_stale_time(Duration::from_secs(30));

	assert_eq!(client.fetch_query(&scope, ()).await, Ok("data"));
	// The host's timer runs the garbage collector; advance time past the
	// stale time and the data goes stale without a fetch.
	host.advance(29);
	assert_eq!(client.fetch_query(&scope, ()).await, Ok("data"));
	assert_eq!(calls.get(), 1, "still fresh by the host's clock");

	host.advance(2);
	assert_eq!(client.fetch_query(&scope, ()).await, Ok("data"));
	assert_eq!(calls.get(), 2, "stale by the host's clock, refetched");
}

#[tokio::test]
async fn the_hosts_timer_paces_retries() {
	common::init();
	let host = HostClock::shared();
	let client = QueryClient::new();
	let attempts = Counter::new();
	let scope: QueryScope<(), u32, Boom> = {
		let attempts = attempts.clone();
		QueryScope::new(move |()| {
			let attempts = attempts.clone();
			async move {
				if attempts.hit() < 3 {
					Err(Boom("flaky"))
				} else {
					Ok(3)
				}
			}
		})
	}
	.with_name("host-retries")
	.with_options(
		QueryOptions::new()
			.with_retry(Retry::count(3))
			.with_retry_delay(Duration::from_millis(25)),
	);

	let outcome = client.fetch_query(&scope, ()).await;
	assert_eq!(outcome, Ok(3));
	assert_eq!(attempts.get(), 3);
	// The retry delays were handed to the host's timer, not slept natively:
	// the whole retry ladder finished without a single real sleep. The
	// cache's cleanup timer runs on the host's timer too.
	let retry_delays = host
		.waited_for()
		.into_iter()
		.filter(|wait| *wait < Duration::from_secs(1))
		.collect::<Vec<_>>();
	assert_eq!(retry_delays, vec![Duration::from_millis(25); 2]);
}

#[tokio::test]
async fn focus_and_network_come_from_the_host() {
	common::init();
	let _host = HostClock::shared();
	let client = QueryClient::new()
		.with_defaults(QueryDefaults::new().with_network_mode(NetworkMode::Online));
	// `provide()` does this in a component tree; a headless host mounts the
	// client itself to wire it to focus and network events.
	client.mount();
	let calls = Counter::new();
	let scope: QueryScope<(), (), Boom> = {
		let calls = calls.clone();
		QueryScope::new(move |()| {
			let calls = calls.clone();
			async move {
				calls.hit();
				Ok(())
			}
		})
		.with_name("host-events")
	};

	// The host reports the network gone before anything fetches.
	OnlineManager::set_online(false);
	let owner = Owner::new();
	let query =
		owner.with(|| use_query_in(client.clone(), scope, || (), QueryObserverOptions::new()));
	settle().await;
	assert!(query.is_paused());
	assert_eq!(calls.get(), 0);

	OnlineManager::set_online(true);
	wait_for("resumed", || query.is_success()).await;

	// Refetch on focus is the host's to report too.
	let before = calls.get();
	FocusManager::set_focused(Some(false));
	FocusManager::set_focused(Some(true));
	wait_for("refetched on focus", || calls.get() > before).await;
	FocusManager::set_focused(None);

	owner.cleanup();
	client.unmount();
}
