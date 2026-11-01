#![cfg(not(target_arch = "wasm32"))]

//! Query caching, observers, staleness, retries, invalidation, and lifecycle.
mod common;

use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicU32;
use std::sync::atomic::Ordering;
use std::time::Duration;

use common::Boom;
use common::Counter;
use common::Gates;
use common::owned;
use common::quiet_client;
use common::settle;
use common::wait_for;
use leptos::prelude::*;
use leptoskit_query::*;

/// A scope whose fetcher doubles the key and counts calls.
fn doubler(calls: Counter) -> QueryScope<u32, u32, Boom> {
	QueryScope::new(move |key: u32| {
		let calls = calls.clone();
		async move {
			calls.hit();
			tokio::time::sleep(Duration::from_millis(5)).await;
			Ok(key * 2)
		}
	})
	.with_name("doubler")
}

#[tokio::test]
async fn fetch_query_caches_and_deduplicates() {
	let client = quiet_client();
	let calls = Counter::new();
	let scope = doubler(calls.clone()).with_stale_time(Duration::from_secs(60));

	let (a, b) = futures::join!(
		client.fetch_query(&scope, 21),
		client.fetch_query(&scope, 21)
	);
	assert_eq!(a, Ok(42));
	assert_eq!(b, Ok(42));
	assert_eq!(calls.get(), 1, "concurrent callers share one fetch");

	assert_eq!(client.fetch_query(&scope, 21).await, Ok(42));
	assert_eq!(calls.get(), 1, "fresh data is served from the cache");
	assert_eq!(client.get_query_data(&scope, &21), Some(42));
	assert!(client.query_exists(&scope, &21));
	assert!(!client.query_exists(&scope, &22));
}

#[tokio::test]
async fn stale_data_is_fetched_again() {
	let client = quiet_client();
	let calls = Counter::new();
	let scope = doubler(calls.clone()).with_stale_time(Duration::from_millis(20));

	assert_eq!(client.fetch_query(&scope, 1).await, Ok(2));
	tokio::time::sleep(Duration::from_millis(40)).await;
	assert_eq!(client.fetch_query(&scope, 1).await, Ok(2));
	assert_eq!(calls.get(), 2, "stale data triggers a refetch");

	let state = client.get_query_state(&scope, &1).expect("cached");
	assert_eq!(state.meta.data_update_count, 2);
	assert!(state.is_success());
}

#[tokio::test]
async fn ensure_query_data_serves_stale_data_without_fetching() {
	let client = quiet_client();
	let calls = Counter::new();
	let scope = doubler(calls.clone());

	assert_eq!(client.ensure_query_data(&scope, 5).await, Ok(10));
	assert_eq!(client.ensure_query_data(&scope, 5).await, Ok(10));
	assert_eq!(calls.get(), 1);

	assert_eq!(
		client.ensure_query_data_revalidating(&scope, 5).await,
		Ok(10)
	);
	wait_for("background revalidation", || calls.get() == 2).await;
}

#[tokio::test]
async fn fetch_query_surfaces_errors() {
	let client = quiet_client();
	let scope: QueryScope<u32, u32, Boom> =
		QueryScope::new(|_| async { Err(Boom("nope")) }).with_retry(Retry::never());

	assert_eq!(client.fetch_query(&scope, 1).await, Err(Boom("nope")));
	let state = client.get_query_state(&scope, &1).expect("cached");
	assert!(state.is_error());
	assert!(state.is_loading_error());
	assert_eq!(state.meta.error_update_count, 1);
}

#[tokio::test]
async fn observer_loads_then_succeeds() {
	let client = quiet_client();
	let calls = Counter::new();
	let scope = doubler(calls.clone());

	let (owner, query) =
		owned(|| use_query_in(client.clone(), scope, || 4, QueryObserverOptions::new()));

	assert!(query.is_pending());
	assert!(query.is_loading(), "the first fetch starts on mount");
	assert!(query.is_enabled());
	assert_eq!(query.data(), None);

	wait_for("data", || query.is_success()).await;
	assert_eq!(query.data(), Some(8));
	assert!(!query.is_fetching());
	assert!(query.is_fetched());
	assert!(query.is_fetched_after_mount());
	assert!(query.data_updated_at().is_some());
	assert_eq!(calls.get(), 1);

	let result = query.get();
	assert!(result.is_success());
	assert_eq!(result.data, Some(8));
	assert!(!result.is_placeholder_data);
	assert!(result.is_stale, "stale time is zero by default");

	owner.cleanup();
}

#[tokio::test]
async fn key_changes_rebind_and_keep_previous_data() {
	let client = quiet_client();
	let calls = Counter::new();
	let scope = doubler(calls.clone());
	let key = ArcRwSignal::new(1_u32);

	let (owner, query) = {
		let key = key.clone();
		owned(|| {
			use_query_in(
				client.clone(),
				scope,
				move || key.get(),
				QueryObserverOptions::new().keep_previous_data(),
			)
		})
	};
	wait_for("first key", || query.data() == Some(2)).await;

	key.set(10);
	settle().await;
	// The new key is loading, but the previous data stays on screen.
	assert!(query.is_placeholder_data() || query.data() == Some(20));

	if query.is_placeholder_data() {
		assert_eq!(query.data(), Some(2));
		assert!(query.is_success(), "placeholder data counts as success");
	}

	wait_for("second key", || {
		query.data() == Some(20) && !query.is_placeholder_data()
	})
	.await;
	assert_eq!(calls.get(), 2);

	assert_eq!(client.query_infos().len(), 2, "both keys are cached");

	owner.cleanup();
}

#[tokio::test]
async fn optional_keys_disable_the_query() {
	let client = quiet_client();
	let calls = Counter::new();
	let scope = doubler(calls.clone());
	let key = ArcRwSignal::new(None::<u32>);

	let (owner, query) = {
		let key = key.clone();
		owned(|| {
			use_query_in(
				client.clone(),
				scope,
				move || key.get(),
				QueryObserverOptions::new(),
			)
		})
	};
	settle().await;
	assert!(query.is_pending());
	assert!(
		!query.is_loading(),
		"a disabled query is pending but not loading"
	);
	assert!(!query.is_enabled());
	assert_eq!(calls.get(), 0);

	key.set(Some(3));
	wait_for("enabled by key", || query.data() == Some(6)).await;
	assert!(query.is_enabled());

	owner.cleanup();
}

#[tokio::test]
async fn enabled_flag_gates_fetching() {
	let client = quiet_client();
	let calls = Counter::new();
	let scope = doubler(calls.clone());
	let enabled = ArcRwSignal::new(false);

	let (owner, query) = {
		let enabled = enabled.clone();
		owned(|| {
			use_query_in(
				client.clone(),
				scope,
				|| 7,
				QueryObserverOptions::new().with_enabled(move || enabled.get()),
			)
		})
	};
	settle().await;
	assert_eq!(calls.get(), 0, "disabled queries do not fetch on mount");
	assert!(!query.is_enabled());

	enabled.set(true);
	wait_for("enabled", || query.data() == Some(14)).await;
	assert_eq!(calls.get(), 1);

	owner.cleanup();
}

#[tokio::test]
async fn retries_then_succeeds() {
	let client = quiet_client();
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
	.with_name("retried")
	.with_retry(Retry::count(3))
	.with_options(QueryOptions::new().with_retry_delay(Duration::from_millis(1)));

	let (owner, query) =
		owned(|| use_query_in(client.clone(), scope, || (), QueryObserverOptions::new()));
	wait_for("retried", || query.data() == Some(3)).await;
	assert_eq!(attempts.load(Ordering::SeqCst), 3);
	assert_eq!(query.failure_count(), 0, "success resets the failure count");
	assert_eq!(query.failure_reason(), None);
	assert_eq!(query.error(), None);

	owner.cleanup();
}

#[tokio::test]
async fn exhausted_retries_surface_the_error() {
	let client = quiet_client();
	let attempts = Counter::new();
	let scope: QueryScope<(), u32, Boom> = {
		let attempts = attempts.clone();
		QueryScope::new(move |()| {
			let attempts = attempts.clone();
			async move {
				attempts.hit();
				Err(Boom("down"))
			}
		})
	}
	.with_name("exhausted")
	.with_options(
		QueryOptions::new()
			.with_retry(Retry::count(2))
			.with_retry_delay(Duration::from_millis(1)),
	);

	let (owner, query) =
		owned(|| use_query_in(client.clone(), scope, || (), QueryObserverOptions::new()));
	wait_for("errored", || query.is_error()).await;
	assert_eq!(attempts.get(), 3, "one attempt plus two retries");
	assert_eq!(query.error(), Some(Boom("down")));
	assert_eq!(query.failure_count(), 3);
	assert_eq!(query.failure_reason(), Some(Boom("down")));
	assert!(query.is_loading_error());

	assert_eq!(query.result(), Some(Err(Boom("down"))));

	owner.cleanup();
}

#[tokio::test]
async fn retry_decides_per_error() {
	let client = quiet_client();
	let attempts = Counter::new();
	let scope: QueryScope<(), u32, Boom> = {
		let attempts = attempts.clone();
		QueryScope::new(move |()| {
			let attempts = attempts.clone();
			async move {
				attempts.hit();
				Err(Boom("fatal"))
			}
		})
	}
	.with_name("fatal")
	.with_retry(Retry::decide(|_, error: &Boom| error.0 != "fatal"));

	assert_eq!(client.fetch_query(&scope, ()).await, Err(Boom("fatal")));
	assert_eq!(attempts.get(), 1, "a fatal error is not retried");
}

#[tokio::test]
async fn invalidation_refetches_active_queries_only() {
	let client = quiet_client();
	let calls = Counter::new();
	let scope = doubler(calls.clone()).with_stale_time(Duration::from_secs(60));

	// An inactive query: fetched imperatively, nobody observes it.
	assert_eq!(client.fetch_query(&scope, 1).await, Ok(2));
	// An active one.
	let (owner, active) = owned(|| {
		use_query_in(
			client.clone(),
			scope.clone(),
			|| 2,
			QueryObserverOptions::new(),
		)
	});
	wait_for("active loaded", || active.data() == Some(4)).await;
	assert_eq!(calls.get(), 2);

	client.invalidate_scope(&scope).await;
	assert_eq!(calls.get(), 3, "only the observed query refetched");
	let inactive = client.get_query_state(&scope, &1).expect("still cached");
	assert!(
		inactive.meta.is_invalidated,
		"the inactive query is marked stale"
	);
	assert!(!active.is_stale(), "the active query is fresh again");

	// Refetching everything reaches the inactive query too.
	client
		.invalidate_queries_with(QueryFilter::scope(&scope), InvalidateOptions::REFETCH_ALL)
		.await;
	assert_eq!(calls.get(), 5);

	owner.cleanup();
}

#[tokio::test]
async fn tags_invalidate_by_prefix() {
	let client = quiet_client();
	let calls = Counter::new();
	let scope = doubler(calls.clone())
		.with_stale_time(Duration::from_secs(60))
		.with_tags_for(|key| vec![tag!["users", key]]);

	let (owner, (seven, eight)) = owned(|| {
		(
			use_query_in(
				client.clone(),
				scope.clone(),
				|| 7,
				QueryObserverOptions::new(),
			),
			use_query_in(
				client.clone(),
				scope.clone(),
				|| 8,
				QueryObserverOptions::new(),
			),
		)
	});
	wait_for("both loaded", || {
		seven.data().is_some() && eight.data().is_some()
	})
	.await;
	assert_eq!(calls.get(), 2);

	client.invalidate_tag(tag!["users", 7]).await;
	assert_eq!(calls.get(), 3, "only user 7 refetched");

	client.invalidate_tag("users").await;
	assert_eq!(calls.get(), 5, "the prefix reaches both");

	client.invalidate_tag("posts").await;
	assert_eq!(calls.get(), 5, "an unrelated tag matches nothing");

	owner.cleanup();
}

#[tokio::test]
async fn set_and_update_query_data_are_seen_immediately() {
	let client = quiet_client();
	let calls = Counter::new();
	let scope = doubler(calls.clone()).with_stale_time(Duration::from_secs(60));

	client.set_query_data(&scope, 1, 100);
	assert_eq!(client.get_query_data(&scope, &1), Some(100));

	let (owner, query) = owned(|| {
		use_query_in(
			client.clone(),
			scope.clone(),
			|| 1,
			QueryObserverOptions::new(),
		)
	});
	assert_eq!(query.data(), Some(100), "seeded data is fresh, so no fetch");
	settle().await;
	assert_eq!(calls.get(), 0);

	client.update_query_data(&scope, 1, |current| current.map(|value| value + 1));
	assert_eq!(query.data(), Some(101));

	let skipped = client.update_query_data(&scope, 99, |current| current.map(|value| value + 1));
	assert_eq!(skipped, None);
	assert!(
		!client.query_exists(&scope, &99),
		"a `None` updater creates nothing"
	);

	let written = client.update_queries_data(&scope, |_, value| Some(value * 2));
	assert_eq!(written, 1);
	assert_eq!(query.data(), Some(202));
	assert_eq!(client.get_queries_data(&scope), vec![(1, Some(202))]);

	query.set_data(5);
	assert_eq!(query.data(), Some(5));

	owner.cleanup();
}

#[tokio::test]
async fn initial_data_seeds_the_cache() {
	let client = quiet_client();
	let calls = Counter::new();
	let scope = doubler(calls.clone()).with_stale_time(Duration::from_secs(60));

	let (owner, query) = owned(|| {
		use_query_in(
			client.clone(),
			scope.clone(),
			|| 3,
			QueryObserverOptions::new().with_initial_data(999),
		)
	});
	assert_eq!(query.data(), Some(999));
	assert!(query.is_success());
	settle().await;
	assert_eq!(calls.get(), 0, "fresh initial data is not refetched");

	owner.cleanup();
}

#[tokio::test]
async fn placeholder_data_is_not_cached() {
	let client = quiet_client();
	let calls = Counter::new();
	let scope = doubler(calls.clone());

	let (owner, query) = owned(|| {
		use_query_in(
			client.clone(),
			scope.clone(),
			|| 3,
			QueryObserverOptions::new().with_placeholder_data(0),
		)
	});
	assert_eq!(query.data(), Some(0));
	assert!(query.is_placeholder_data());
	assert_eq!(
		client.get_query_data(&scope, &3),
		None,
		"placeholders stay out of the cache"
	);
	wait_for("real data", || query.data() == Some(6)).await;
	assert!(!query.is_placeholder_data());

	owner.cleanup();
}

#[tokio::test]
async fn unobserved_queries_are_garbage_collected() {
	let client = quiet_client();
	let calls = Counter::new();
	let scope = doubler(calls.clone()).with_gc_time(Duration::from_millis(30));

	let (owner, query) = owned(|| {
		use_query_in(
			client.clone(),
			scope.clone(),
			|| 1,
			QueryObserverOptions::new(),
		)
	});
	wait_for("loaded", || query.data().is_some()).await;
	assert_eq!(client.query_infos().len(), 1);

	tokio::time::sleep(Duration::from_millis(60)).await;
	assert_eq!(
		client.query_infos().len(),
		1,
		"observed queries are never collected"
	);

	owner.cleanup();
	wait_for("collected", || client.query_infos().is_empty()).await;
}

#[tokio::test]
async fn removed_queries_are_rebound_by_observers() {
	let client = quiet_client();
	let calls = Counter::new();
	let scope = doubler(calls.clone()).with_stale_time(Duration::from_secs(60));

	let (owner, query) = owned(|| {
		use_query_in(
			client.clone(),
			scope.clone(),
			|| 1,
			QueryObserverOptions::new(),
		)
	});
	wait_for("loaded", || query.data().is_some()).await;

	client.remove_queries(QueryFilter::scope(&scope));
	wait_for("rebound and refetched", || calls.get() == 2).await;
	assert_eq!(client.query_infos().len(), 1);

	owner.cleanup();
}

#[tokio::test]
async fn reset_returns_to_pending_and_refetches() {
	let client = quiet_client();
	let calls = Counter::new();
	let scope = doubler(calls.clone()).with_stale_time(Duration::from_secs(60));

	let (owner, query) = owned(|| {
		use_query_in(
			client.clone(),
			scope.clone(),
			|| 1,
			QueryObserverOptions::new(),
		)
	});
	wait_for("loaded", || query.data().is_some()).await;

	client.reset_queries(QueryFilter::scope(&scope)).await;
	assert_eq!(calls.get(), 2, "active queries refetch after a reset");
	assert_eq!(query.data(), Some(2));
	assert_eq!(query.meta().data_update_count, 1, "counters start over");

	owner.cleanup();
}

/// Fails the first call, then succeeds after `slow` on the second call and
/// at once after that.
fn flaky(calls: Counter, slow: Duration) -> QueryScope<u32, u32, Boom> {
	QueryScope::new(move |key: u32| {
		let calls = calls.clone();
		async move {
			match calls.hit() {
				1 => Err(Boom("first")),
				2 => {
					tokio::time::sleep(slow).await;
					Ok(key * 2)
				}
				_ => Ok(key * 2),
			}
		}
	})
	.with_name("flaky")
	.with_retry(Retry::never())
	.with_stale_time(Duration::from_secs(60))
}

#[tokio::test]
async fn reset_during_a_fetch_starts_over_instead_of_joining_it() {
	let client = quiet_client();
	let calls = Counter::new();
	let scope = flaky(calls.clone(), Duration::from_millis(200));

	let (owner, query) = owned(|| {
		use_query_in(
			client.clone(),
			scope.clone(),
			|| 1,
			QueryObserverOptions::new(),
		)
	});
	wait_for("first fetch failed", || query.is_error()).await;
	drop(query.refetch());
	assert!(query.is_fetching());
	// Let the refetch reach the slow server, so the reset retires a fetch
	// that is really in flight.
	settle().await;
	assert_eq!(calls.get(), 2);

	client.reset_queries(QueryFilter::scope(&scope)).await;
	assert_eq!(calls.get(), 3, "the reset fetched afresh");
	assert_eq!(query.data(), Some(2));
	settle().await;
	assert_eq!(
		query.error(),
		None,
		"the cancelled fetch does not restore the error from before the reset"
	);
	assert_eq!(query.failure_count(), 0);

	owner.cleanup();
}

#[tokio::test]
async fn a_refetch_right_after_a_cancel_starts_a_new_fetch() {
	let client = quiet_client();
	let calls = Counter::new();
	let scope = doubler(calls.clone()).with_stale_time(Duration::from_secs(60));

	let (owner, query) = owned(|| {
		use_query_in(
			client.clone(),
			scope.clone(),
			|| 4,
			QueryObserverOptions::new(),
		)
	});
	assert!(query.is_fetching());
	client.cancel_queries(QueryFilter::scope(&scope));
	client.refetch_queries(QueryFilter::scope(&scope)).await;
	// Joining the cancelled fetch would have left the query without data.
	assert_eq!(query.data(), Some(8), "the cancelled fetch was not joined");
	assert_eq!(
		calls.get(),
		1,
		"and, cancelled before its task ran, it never called the fetcher"
	);

	owner.cleanup();
}

#[tokio::test]
async fn ensure_query_data_fetches_again_after_an_error() {
	let client = quiet_client();
	let calls = Counter::new();
	let scope = flaky(calls.clone(), Duration::ZERO);

	assert_eq!(client.fetch_query(&scope, 3).await, Err(Boom("first")));
	assert_eq!(
		client.ensure_query_data(&scope, 3).await,
		Ok(6),
		"a cached error is not data"
	);
	assert_eq!(calls.get(), 2);
}

#[tokio::test]
async fn cancel_restores_the_previous_state() {
	let client = quiet_client();
	let scope: QueryScope<u32, u32, Boom> = QueryScope::new(|key: u32| {
		async move {
			tokio::time::sleep(Duration::from_millis(200)).await;
			Ok(key)
		}
	})
	.with_stale_time(Duration::from_secs(60));

	let (owner, query) = owned(|| {
		use_query_in(
			client.clone(),
			scope.clone(),
			|| 1,
			QueryObserverOptions::new(),
		)
	});
	settle().await;
	assert!(query.is_fetching());
	assert_eq!(client.fetching_count(), 1);

	client.cancel_queries(QueryFilter::scope(&scope));
	wait_for("idle", || !query.is_fetching()).await;
	assert!(query.is_pending());
	assert_eq!(client.fetching_count(), 0);

	owner.cleanup();
}

#[tokio::test]
async fn refetch_restarts_and_resolves() {
	let client = quiet_client();
	let calls = Counter::new();
	let scope = doubler(calls.clone()).with_stale_time(Duration::from_secs(60));

	let (owner, query) = owned(|| {
		use_query_in(
			client.clone(),
			scope.clone(),
			|| 1,
			QueryObserverOptions::new(),
		)
	});
	wait_for("loaded", || query.data().is_some()).await;

	let outcome = query.refetch().await;
	assert_eq!(outcome, Some(Ok(2)));
	assert_eq!(calls.get(), 2);
	assert!(query.is_refetching() || !query.is_fetching());

	query.invalidate();
	wait_for("invalidated refetch", || calls.get() == 3).await;

	owner.cleanup();
}

#[tokio::test]
async fn refetch_interval_polls_while_observed() {
	let client = quiet_client();
	let calls = Counter::new();
	let scope = doubler(calls.clone())
		.with_stale_time(Duration::from_secs(60))
		.with_refetch_interval(Duration::from_millis(15));

	let (owner, query) = owned(|| {
		use_query_in(
			client.clone(),
			scope.clone(),
			|| 1,
			QueryObserverOptions::new(),
		)
	});
	wait_for("polled", || calls.get() >= 3).await;
	assert_eq!(query.data(), Some(2));

	owner.cleanup();
	let after_cleanup = calls.get();
	tokio::time::sleep(Duration::from_millis(60)).await;
	assert!(
		calls.get() <= after_cleanup + 1,
		"polling stops once the observer unmounts"
	);
}

#[tokio::test]
async fn awaiting_a_handle_resolves_with_data() {
	let client = quiet_client();
	let calls = Counter::new();
	let scope = doubler(calls.clone());

	let (owner, query) = owned(|| {
		use_query_in(
			client.clone(),
			scope.clone(),
			|| 21,
			QueryObserverOptions::new(),
		)
	});
	assert_eq!((&query).await, Ok(42));
	assert_eq!(query.await, Ok(42));
	assert_eq!(calls.get(), 1);

	owner.cleanup();
}

#[tokio::test]
async fn awaiting_a_disabled_handle_still_fetches() {
	let client = quiet_client();
	let calls = Counter::new();
	let scope = doubler(calls.clone());

	let (owner, query) = owned(|| {
		use_query_in(
			client.clone(),
			scope.clone(),
			|| 1,
			QueryObserverOptions::new().with_enabled(|| false),
		)
	});
	assert_eq!(query.wait().await, Ok(2));
	assert_eq!(calls.get(), 1);

	owner.cleanup();
}

#[tokio::test]
async fn structural_sharing_skips_equal_data() {
	let client = quiet_client();
	let scope: QueryScope<(), Vec<u32>, Boom> =
		QueryScope::new(|()| async { Ok(vec![1, 2, 3]) }).with_structural_sharing();

	let notifications = Arc::new(AtomicU32::new(0));
	let (owner, query) = owned(|| {
		let query = use_query_in(
			client.clone(),
			scope.clone(),
			|| (),
			QueryObserverOptions::new(),
		);
		let tracked = query;
		let notifications = Arc::clone(&notifications);
		Effect::new_isomorphic(move |_| {
			let _ = tracked.data();
			notifications.fetch_add(1, Ordering::SeqCst);
		});
		query
	});
	wait_for("loaded", || query.data().is_some()).await;
	let before = notifications.load(Ordering::SeqCst);

	assert_eq!(query.refetch().await, Some(Ok(vec![1, 2, 3])));
	settle().await;
	assert_eq!(
		notifications.load(Ordering::SeqCst),
		before,
		"equal data does not wake data subscribers"
	);
	assert_eq!(
		query.meta().data_update_count,
		2,
		"but the fetch is recorded"
	);

	owner.cleanup();
}

#[tokio::test]
async fn select_derives_a_signal() {
	let client = quiet_client();
	let scope: QueryScope<(), Vec<u32>, Boom> = QueryScope::new(|()| async { Ok(vec![1, 2, 3]) });

	let (owner, (query, len)) = owned(|| {
		let query = use_query_in(
			client.clone(),
			scope.clone(),
			|| (),
			QueryObserverOptions::new(),
		);
		let len = query.select(Vec::len);
		(query, len)
	});
	wait_for("loaded", || query.data().is_some()).await;
	assert_eq!(len.get(), Some(3));
	assert_eq!(query.with_data(|data| data.iter().sum::<u32>()), Some(6));

	owner.cleanup();
}

#[tokio::test]
async fn use_queries_follows_a_list_of_keys() {
	let client = quiet_client();
	let calls = Counter::new();
	let scope = doubler(calls.clone()).with_stale_time(Duration::from_secs(60));
	let keys = ArcRwSignal::new(vec![1_u32, 2]);

	let (owner, list) = {
		let keys = keys.clone();
		owned(|| {
			let client = client.clone();
			provide_context(client.clone());
			use_queries(scope.clone(), move || keys.get())
		})
	};
	assert_eq!(list.len(), 2);
	wait_for("both loaded", || list.all_data().is_some()).await;
	assert_eq!(list.all_data(), Some(vec![2, 4]));
	assert!(list.is_success());

	keys.set(vec![1, 2, 3]);
	wait_for("third loaded", || list.all_data() == Some(vec![2, 4, 6])).await;
	assert_eq!(calls.get(), 3);

	keys.set(vec![3]);
	settle().await;
	assert_eq!(list.len(), 1);
	assert_eq!(list.all_data(), Some(vec![6]));

	owner.cleanup();
}

#[tokio::test]
async fn window_focus_refetches_stale_queries() {
	common::init();
	let client = QueryClient::new()
		.with_defaults(QueryDefaults::new().with_network_mode(NetworkMode::Always));
	client.mount();
	let calls = Counter::new();
	let scope = doubler(calls.clone());

	let (owner, query) = owned(|| {
		use_query_in(
			client.clone(),
			scope.clone(),
			|| 1,
			QueryObserverOptions::new(),
		)
	});
	wait_for("loaded", || query.data().is_some()).await;

	FocusManager::set_focused(Some(false));
	FocusManager::set_focused(Some(true));
	wait_for("refetched on focus", || calls.get() == 2).await;
	FocusManager::set_focused(None);

	owner.cleanup();
	client.unmount();
}

#[tokio::test]
async fn offline_pauses_fetches_until_the_network_returns() {
	common::init();
	let client = QueryClient::new().with_defaults(
		QueryDefaults::new()
			.with_refetch_on_window_focus(RefetchOn::Never)
			.with_refetch_on_reconnect(RefetchOn::Never),
	);
	let calls = Counter::new();
	let scope = doubler(calls.clone());

	OnlineManager::set_online(false);
	let (owner, query) = owned(|| {
		use_query_in(
			client.clone(),
			scope.clone(),
			|| 1,
			QueryObserverOptions::new(),
		)
	});
	settle().await;
	assert!(query.is_paused());
	assert_eq!(calls.get(), 0);
	assert_eq!(client.fetching_count(), 0, "a paused fetch is not fetching");

	OnlineManager::set_online(true);
	wait_for("resumed", || query.data() == Some(2)).await;

	owner.cleanup();
}

#[tokio::test]
async fn dehydrate_and_hydrate_round_trip() {
	let client = quiet_client();
	let calls = Counter::new();
	let scope = doubler(calls.clone())
		.with_name("doubler")
		.with_stale_time(Duration::from_secs(60))
		.with_ssr();

	assert_eq!(client.fetch_query(&scope, 4).await, Ok(8));
	let state = client.dehydrate();
	assert_eq!(state.queries.len(), 1);
	assert_eq!(state.queries[0].scope_name, "doubler");
	let json = serde_json::to_string(&state).expect("serializes");
	let restored: DehydratedState = serde_json::from_str(&json).expect("deserializes");

	let other = quiet_client();
	other.hydrate(restored);
	assert!(
		!other.query_exists(&scope, &4),
		"hydration waits for the query to exist"
	);

	let (owner, query) = owned(|| {
		use_query_in(
			other.clone(),
			scope.clone(),
			|| 4,
			QueryObserverOptions::new(),
		)
	});
	assert_eq!(
		query.data(),
		Some(8),
		"the hydrated data is there on first read"
	);
	settle().await;
	assert_eq!(calls.get(), 1, "fresh hydrated data is not fetched again");

	owner.cleanup();
}

#[tokio::test]
async fn cache_events_are_observable() {
	let client = quiet_client();
	let calls = Counter::new();
	let scope = doubler(calls.clone());
	let added = Arc::new(AtomicBool::new(false));
	let updated = Arc::new(AtomicBool::new(false));
	let subscription = {
		let added = Arc::clone(&added);
		let updated = Arc::clone(&updated);
		client.subscribe(move |notification| {
			match notification.event {
				CacheEvent::Added => added.store(true, Ordering::SeqCst),
				CacheEvent::Updated => updated.store(true, Ordering::SeqCst),
				_ => {}
			}
		})
	};

	assert_eq!(client.fetch_query(&scope, 1).await, Ok(2));
	assert!(added.load(Ordering::SeqCst));
	assert!(updated.load(Ordering::SeqCst));
	let info = &client.query_infos()[0];
	assert_eq!(info.key, "1");
	assert_eq!(info.status, QueryStatus::Success);

	drop(subscription);
	client.clear();
	assert!(client.query_infos().is_empty());
}

#[tokio::test]
async fn filters_narrow_by_key_predicate_and_status() {
	let client = quiet_client();
	let calls = Counter::new();
	let scope = doubler(calls.clone()).with_stale_time(Duration::from_secs(60));

	for key in 1..=4 {
		assert_eq!(client.fetch_query(&scope, key).await, Ok(key * 2));
	}

	client.remove_queries(QueryFilter::keys_where(&scope, |key| key % 2 == 0));
	let mut remaining: Vec<u32> = client
		.get_queries_data(&scope)
		.into_iter()
		.map(|(key, _)| key)
		.collect();
	remaining.sort_unstable();
	assert_eq!(remaining, vec![1, 3]);

	assert_eq!(
		client.fetching_count_matching(QueryFilter::scope(&scope)),
		0
	);
	client.remove_queries(QueryFilter::all().inactive());
	assert!(client.query_infos().is_empty());
}

// ---- lifecycle -------------------------------------------------------------
#[tokio::test]
async fn use_queries_keeps_every_handle_live_as_keys_grow() {
	let client = quiet_client();
	let calls = Counter::new();
	let scope = doubler(calls.clone()).with_stale_time(Duration::from_secs(60));
	let keys = ArcRwSignal::new(vec![1_u32, 2]);

	let (owner, list) = {
		let keys = keys.clone();
		owned(|| {
			provide_context(client.clone());
			use_queries(scope.clone(), move || keys.get())
		})
	};
	wait_for("two loaded", || list.all_data() == Some(vec![2, 4])).await;
	// Handles added by a later run of the key effect must outlive the next
	// run, which disposes everything the previous run owned.
	keys.set(vec![1, 2, 3]);
	wait_for("three loaded", || list.all_data() == Some(vec![2, 4, 6])).await;
	keys.set(vec![1, 2, 3, 4]);
	wait_for("four loaded", || list.all_data() == Some(vec![2, 4, 6, 8])).await;
	assert!(
		list.handles().iter().all(|handle| handle.info().is_some()),
		"every handle is live"
	);
	assert!(
		client.query_infos().iter().all(|info| info.observers == 1),
		"one observer per query"
	);

	owner.cleanup();
	let infos = client.query_infos();
	assert_eq!(infos.len(), 4);
	assert!(
		infos.iter().all(|info| info.observers == 0),
		"unmounting releases every observer: {infos:?}"
	);
}

#[tokio::test]
async fn use_queries_rebinds_after_its_queries_are_removed() {
	let client = quiet_client();
	let calls = Counter::new();
	let scope = doubler(calls.clone()).with_stale_time(Duration::from_secs(60));

	let (owner, list) = owned(|| {
		provide_context(client.clone());
		use_queries(scope.clone(), || vec![1_u32, 2])
	});
	wait_for("loaded", || list.all_data().is_some()).await;
	assert_eq!(calls.get(), 2);

	client.remove_queries(QueryFilter::scope(&scope));
	wait_for("refetched into fresh queries", || calls.get() == 4).await;
	wait_for("loaded again", || list.all_data() == Some(vec![2, 4])).await;
	let infos = client.query_infos();
	assert_eq!(infos.len(), 2);
	assert!(infos.iter().all(|info| info.observers == 1));

	owner.cleanup();
}

#[tokio::test]
async fn a_reset_before_a_ready_fetch_runs_leaves_the_query_idle() {
	let client = quiet_client();
	let scope: QueryScope<u32, u32, Boom> =
		QueryScope::new(|key: u32| async move { Ok(key) }).with_retry(Retry::never());
	assert_eq!(client.fetch_query(&scope, 1).await, Ok(1));

	// The refetch starts synchronously; its task has not run yet.
	drop(client.refetch_queries(QueryFilter::scope(&scope)));
	assert_eq!(client.fetching_count(), 1);
	drop(client.reset_queries(QueryFilter::scope(&scope)));
	settle().await;

	let state = client.get_query_state(&scope, &1).expect("still cached");
	assert_eq!(state.fetch_status, FetchStatus::Idle);
	assert_eq!(state.data, None, "the retired fetch wrote nothing");
	assert_eq!(client.fetching_count(), 0);
}

#[tokio::test]
async fn a_reset_before_a_pending_fetch_runs_leaves_the_query_idle() {
	let client = quiet_client();
	let gates = Gates::new();
	let scope = gates.scope("reset-pending");
	client.set_query_data(&scope, 1, 0);

	drop(client.refetch_queries(QueryFilter::scope(&scope)));
	drop(client.reset_queries(QueryFilter::scope(&scope)));
	settle().await;

	let state = client.get_query_state(&scope, &1).expect("still cached");
	assert_eq!(state.fetch_status, FetchStatus::Idle);
	assert_eq!(state.data, None);
	assert_eq!(client.fetching_count(), 0);
	assert_eq!(
		gates.calls(),
		0,
		"a fetch retired before it ran never starts"
	);
}

#[tokio::test]
async fn a_superseded_fetch_never_lands() {
	let client = quiet_client();
	let gates = Gates::new();
	let scope = gates.scope("superseded");
	client.set_query_data(&scope, 1, 0);

	// Fetch A is waiting on the server.
	drop(client.refetch_queries(QueryFilter::scope(&scope)));
	settle().await;
	assert_eq!(gates.calls(), 1);
	// A's response arrives in the same tick a restart replaces it with B.
	gates.answer(0, Ok(1));
	drop(client.refetch_queries(QueryFilter::scope(&scope)));
	settle().await;
	assert_eq!(gates.calls(), 2);
	assert_eq!(
		client.get_query_data(&scope, &1),
		Some(0),
		"the superseded response is discarded"
	);

	assert_eq!(client.fetching_count(), 1, "B is still in flight");

	gates.answer(1, Ok(2));
	settle().await;
	let state = client.get_query_state(&scope, &1).expect("cached");
	assert_eq!(state.data, Some(2), "the newest fetch wins");
	assert_eq!(state.fetch_status, FetchStatus::Idle);
	assert_eq!(client.fetching_count(), 0);
}

#[tokio::test]
async fn a_response_racing_cancel_queries_keeps_the_optimistic_update() {
	let client = quiet_client();
	let gates = Gates::new();
	let scope = gates.scope("optimistic");
	client.set_query_data(&scope, 1, 0);
	drop(client.refetch_queries(QueryFilter::scope(&scope)));
	settle().await;

	// The response is ready, but the fetch task has not seen it yet when an
	// optimistic update cancels the fetch and writes its own value.
	gates.answer(0, Ok(1));
	client.cancel_queries(QueryFilter::scope(&scope));
	client.set_query_data(&scope, 1, 99);
	settle().await;

	let state = client.get_query_state(&scope, &1).expect("cached");
	assert_eq!(state.data, Some(99));
	assert_eq!(state.fetch_status, FetchStatus::Idle);
	assert_eq!(client.fetching_count(), 0);
}

#[tokio::test(start_paused = true)]
async fn seeded_queries_are_garbage_collected() {
	let client = quiet_client();
	let scope = doubler(Counter::new()).with_gc_time(Duration::from_millis(30));
	client.set_query_data(&scope, 1, 10);
	client.set_query_data_at(&scope, 2, 20, now_ms());
	client.update_query_data(&scope, 3, |_| Some(30));
	assert_eq!(client.query_infos().len(), 3);

	tokio::time::sleep(Duration::from_millis(60)).await;
	settle().await;
	assert!(
		client.query_infos().is_empty(),
		"nothing observes seeded data, so it is collected"
	);
}

#[tokio::test(start_paused = true)]
async fn removing_an_observed_query_never_evicts_its_replacement_later() {
	let client = quiet_client();
	let calls = Counter::new();
	// The interval loop of the first binding keeps the detached query alive
	// until its next tick, long after that query's own cleanup comes due.
	let scope = doubler(calls.clone())
		.with_stale_time(Duration::from_secs(60))
		.with_gc_time(Duration::from_millis(30))
		.with_refetch_interval(Duration::from_secs(1));

	let (owner, query) = owned(|| {
		use_query_in(
			client.clone(),
			scope.clone(),
			|| 1,
			QueryObserverOptions::new(),
		)
	});
	wait_for("loaded", || query.data().is_some()).await;
	client.remove_queries(QueryFilter::scope(&scope));
	wait_for("rebound and refetched", || {
		query.data().is_some() && calls.get() == 2
	})
	.await;

	// The detached query lost its last observer when the handle rebound, so
	// its cleanup comes due after the gc time, well before the next tick.
	tokio::time::sleep(Duration::from_millis(100)).await;
	settle().await;
	assert_eq!(
		calls.get(),
		2,
		"the replacement was neither evicted nor refetched"
	);
	let infos = client.query_infos();
	assert_eq!(infos.len(), 1);
	assert_eq!(infos[0].observers, 1);

	owner.cleanup();
}

#[tokio::test(start_paused = true)]
async fn rescheduled_cleanups_keep_one_timer_per_query() {
	let client = quiet_client();
	// A zero stale time, so every call fetches and reschedules the cleanup.
	let scope: QueryScope<u32, u32, Boom> = QueryScope::new(|key: u32| async move { Ok(key) });

	for _ in 0..20 {
		assert_eq!(client.fetch_query(&scope, 1).await, Ok(1));
	}

	settle().await;
	assert_eq!(
		tokio::runtime::Handle::current()
			.metrics()
			.num_alive_tasks(),
		1,
		"one pending cleanup, not one per fetch"
	);
}

/// A key whose hash ignores its value, so every key collides.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Colliding(u32);

impl std::hash::Hash for Colliding {
	fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
		state.write_u8(0);
	}
}

#[tokio::test]
async fn keys_with_equal_hashes_are_separate_queries() {
	let client = quiet_client();
	let scope: QueryScope<Colliding, u32, Boom> =
		QueryScope::new(|key: Colliding| async move { Ok(key.0) })
			.with_stale_time(Duration::from_secs(60));
	assert_eq!(client.fetch_query(&scope, Colliding(1)).await, Ok(1));
	assert_eq!(client.fetch_query(&scope, Colliding(2)).await, Ok(2));
	assert_eq!(client.get_query_data(&scope, &Colliding(1)), Some(1));
	assert_eq!(client.get_query_data(&scope, &Colliding(2)), Some(2));
	assert_eq!(client.query_infos().len(), 2);

	drop(client.invalidate_queries_with(
		QueryFilter::key(&scope, &Colliding(1)),
		InvalidateOptions::MARK_ONLY,
	));
	let invalidated = |key| {
		client
			.get_query_state(&scope, &Colliding(key))
			.expect("cached")
			.meta
			.is_invalidated
	};
	assert!(invalidated(1));
	assert!(
		!invalidated(2),
		"a key filter matches one key, not one hash"
	);

	client.remove_queries(QueryFilter::key(&scope, &Colliding(1)));
	assert!(!client.query_exists(&scope, &Colliding(1)));
	assert!(client.query_exists(&scope, &Colliding(2)));
}

#[tokio::test]
async fn invalidating_inside_an_effect_does_not_subscribe_to_enabled() {
	let client = quiet_client();
	let scope = doubler(Counter::new()).with_stale_time(Duration::from_secs(60));
	let enabled = ArcRwSignal::new(true);
	let runs = Arc::new(AtomicU32::new(0));

	let (owner, query) = {
		let enabled = enabled.clone();
		let runs = Arc::clone(&runs);
		owned(|| {
			let query = use_query_in(
				client.clone(),
				scope.clone(),
				|| 1,
				QueryObserverOptions::new().with_enabled(move || enabled.get()),
			);
			let client = client.clone();
			let scope = scope.clone();
			Effect::new_isomorphic(move |_| {
				runs.fetch_add(1, Ordering::SeqCst);
				drop(client.invalidate_queries(QueryFilter::scope(&scope)));
			});
			query
		})
	};
	wait_for("loaded", || query.data().is_some()).await;
	settle().await;
	let before = runs.load(Ordering::SeqCst);

	enabled.set(false);
	settle().await;
	assert_eq!(
		runs.load(Ordering::SeqCst),
		before,
		"an observer's enabled flag is not a dependency of whoever invalidates"
	);

	owner.cleanup();
}

#[tokio::test]
async fn hydrated_data_shows_at_once_and_a_zero_stale_time_refetches_it() {
	let client = quiet_client();
	let calls = Counter::new();
	let scope = doubler(calls.clone()).with_name("hydrated").with_ssr();
	assert_eq!(client.fetch_query(&scope, 4).await, Ok(8));

	let other = quiet_client();
	other.hydrate(client.dehydrate());
	let (owner, query) = owned(|| {
		use_query_in(
			other.clone(),
			scope.clone(),
			|| 4,
			QueryObserverOptions::new(),
		)
	});
	assert_eq!(query.data(), Some(8), "the hydrated data is there at once");
	assert!(
		query.is_refetching(),
		"but with no stale time it is already stale, so it refetches"
	);
	wait_for("background refetch", || calls.get() == 2).await;

	owner.cleanup();
}
