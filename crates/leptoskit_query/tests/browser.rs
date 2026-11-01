//! Browser-only behaviour: `setTimeout` timers, `Date.now()`, the document's
//! visibility, `navigator.onLine`, and `BroadcastChannel` cross-tab sync.
//!
//! Runs under `wasm-bindgen-test-runner` with a webdriver, as `test:browser`
//! does.

#![cfg(all(target_arch = "wasm32", feature = "browser"))]

use std::sync::Arc;
use std::sync::atomic::AtomicU32;
use std::sync::atomic::Ordering;
use std::time::Duration;

use leptos::prelude::*;
use leptoskit_query::*;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

#[derive(Clone, Debug, PartialEq, Eq)]
struct Boom(&'static str);

fn init() -> QueryClient {
	console_error_panic_hook::set_once();
	let _ = any_spawner::Executor::init_wasm_bindgen();
	QueryClient::new()
}

/// Polls `condition` until it holds, failing after a second — on the
/// browser's own timer.
async fn wait_for(label: &str, condition: impl Fn() -> bool) {
	for _ in 0..200 {
		if condition() {
			return;
		}

		sleep(Duration::from_millis(5)).await;
	}

	panic!("timed out waiting for: {label}");
}

#[wasm_bindgen_test]
async fn the_clock_is_the_wall_clock() {
	assert!(
		now_ms() > 1_600_000_000_000,
		"epoch milliseconds from Date.now()"
	);
}

#[wasm_bindgen_test]
async fn queries_load_on_the_browser_timer() {
	let client = init();
	let scope: QueryScope<u32, u32, Boom> = QueryScope::new(|key: u32| {
		async move {
			// A real `setTimeout` round trip.
			sleep(Duration::from_millis(10)).await;
			Ok(key * 2)
		}
	});

	let owner = Owner::new();
	let query =
		owner.with(|| use_query_in(client.clone(), scope, || 21, QueryObserverOptions::new()));
	assert!(query.is_loading());
	assert_eq!(query.wait().await, Ok(42));
	assert!(!query.is_fetching());
	owner.cleanup();
}

#[wasm_bindgen_test]
async fn retries_are_paced_by_set_timeout() {
	let client = init();
	let attempts = Arc::new(AtomicU32::new(0));
	let scope: QueryScope<(), u32, Boom> = {
		let attempts = Arc::clone(&attempts);
		QueryScope::new(move |()| {
			let attempts = Arc::clone(&attempts);
			async move {
				let attempt = attempts.fetch_add(1, Ordering::SeqCst) + 1;
				if attempt < 3 {
					Err(Boom("flaky"))
				} else {
					Ok(attempt)
				}
			}
		})
	}
	.with_name("browser-retries")
	.with_retry(Retry::count(3))
	.with_retry_delay(Duration::from_millis(5));

	assert_eq!(client.fetch_query(&scope, ()).await, Ok(3));
	assert_eq!(attempts.load(Ordering::SeqCst), 3);
}

#[wasm_bindgen_test]
async fn the_document_drives_focus_and_the_navigator_drives_online() {
	let client = init();
	client.mount();
	// Idempotent, and harmless to call again.
	install_browser_listeners();

	assert!(FocusManager::is_focused(), "the test page is visible");
	assert!(OnlineManager::is_online(), "the test runner is online");

	// Overrides still win over detection.
	FocusManager::set_focused(Some(false));
	assert!(!FocusManager::is_focused());
	FocusManager::set_focused(None);
	assert!(FocusManager::is_focused());
	client.unmount();
}

#[wasm_bindgen_test]
async fn tabs_share_cache_writes_over_a_broadcast_channel() {
	// Two clients in one page stand in for two tabs: a `BroadcastChannel`
	// delivers to every other channel of the same name, in-page included.
	let left = init();
	let right = QueryClient::new();
	let left_sync = left.broadcast("leptoskit-query-browser-test");
	let _right_sync = right.broadcast("leptoskit-query-browser-test");
	assert!(left_sync.is_connected());

	let scope: QueryScope<(), String, Boom> =
		QueryScope::new(|()| async { Ok("fetched".to_owned()) })
			.with_name("browser-shared")
			.with_stale_time(Duration::from_secs(60))
			.with_ssr();

	left.set_query_data(&scope, (), "from the other tab".to_owned());
	wait_for("the peer sees the write", || {
		right.get_query_data(&scope, &()) == Some("from the other tab".to_owned())
	})
	.await;
}

#[wasm_bindgen_test]
async fn interval_refetching_runs_on_the_browser_timer() {
	let client = init();
	let calls = Arc::new(AtomicU32::new(0));
	let scope: QueryScope<(), u32, Boom> = {
		let calls = Arc::clone(&calls);
		QueryScope::new(move |()| {
			let calls = Arc::clone(&calls);
			async move { Ok(calls.fetch_add(1, Ordering::SeqCst)) }
		})
	}
	.with_name("browser-interval")
	.with_stale_time(Duration::from_secs(60))
	.with_refetch_interval(Duration::from_millis(10));

	let owner = Owner::new();
	let query =
		owner.with(|| use_query_in(client.clone(), scope, || (), QueryObserverOptions::new()));
	wait_for("polled three times", || calls.load(Ordering::SeqCst) >= 3).await;
	assert!(query.is_success());
	owner.cleanup();
}
