#![cfg(not(target_arch = "wasm32"))]

//! The public surface beyond the main flows: builders, accessors, filters,
//! context hooks, and the debug output devtools and logs rely on.

mod common;

use std::sync::Arc;
use std::sync::atomic::AtomicU32;
use std::sync::atomic::Ordering;
use std::time::Duration;

use common::Boom;
use common::Counter;
use common::owned;
use common::quiet_client;
use common::settle;
use common::wait_for;
use leptos::prelude::*;
use leptoskit_query::*;

fn doubler(calls: Counter) -> QueryScope<u32, u32, Boom> {
	QueryScope::new(move |key: u32| {
		let calls = calls.clone();
		async move {
			calls.hit();
			tokio::time::sleep(Duration::from_millis(3)).await;
			Ok(key * 2)
		}
	})
	.with_stale_time(Duration::from_secs(60))
}

fn failing(calls: Counter) -> QueryScope<u32, u32, Boom> {
	QueryScope::new(move |_key: u32| {
		let calls = calls.clone();
		async move {
			calls.hit();
			Err(Boom("down"))
		}
	})
	.with_retry(false)
}

#[test]
fn query_state_predicates_follow_status_and_fetch_status() {
	let mut state: QueryState<u32, Boom> = QueryState {
		data: None,
		error: None,
		fetch_status: FetchStatus::Fetching,
		failure_reason: None,
		meta: QueryMeta::default(),
	};
	assert!(state.is_pending());
	assert!(state.is_fetching());
	assert!(state.is_loading());
	assert!(!state.is_refetching());
	assert!(!state.is_paused());
	assert!(!state.is_fetched());
	assert_eq!(state.as_result(), None);

	state.data = Some(1);
	state.meta.data_updated_at = Some(1);
	state.meta.data_update_count = 1;
	assert!(state.is_refetching());
	assert!(state.is_fetched());
	assert_eq!(state.as_result(), Some(Ok(&1)));

	state.error = Some(Boom("late"));
	state.fetch_status = FetchStatus::Paused;
	assert!(state.is_paused());
	assert!(state.is_refetch_error());
	assert_eq!(state.as_result(), Some(Err(&Boom("late"))));
}

#[test]
fn retry_policies_and_delays() {
	assert!(RetryPolicy::Forever.should_retry(1_000));
	let forever: Retry<Boom> = Retry::forever();
	assert!(forever.should_retry(99, &Boom("x")));
	let from_bool: Retry<Boom> = true.into();
	assert!(from_bool.should_retry(5, &Boom("x")));
	let from_count: Retry<Boom> = 2_u32.into();
	assert!(from_count.should_retry(1, &Boom("x")));
	assert!(!from_count.should_retry(2, &Boom("x")));
	let from_policy: Retry<Boom> = RetryPolicy::Never.into();
	assert!(!from_policy.should_retry(0, &Boom("x")));
	let decide: Retry<Boom> = Retry::decide(|count, error: &Boom| count < 1 && error.0 == "retry");
	assert!(decide.should_retry(0, &Boom("retry")));
	assert!(format!("{forever:?}").starts_with("Policy"));
	assert_eq!(format!("{decide:?}"), "Decide(..)");
	assert!(format!("{:?}", decide.clone()).contains("Decide"));

	let fixed: RetryDelay<Boom> = Duration::from_millis(7).into();
	assert_eq!(fixed.delay(3, &Boom("x")), Duration::from_millis(7));
	let backoff: RetryDelay<Boom> =
		Backoff::exponential(Duration::from_millis(10), Duration::from_millis(15)).into();
	assert_eq!(backoff.delay(5, &Boom("x")), Duration::from_millis(15));
	let decided = RetryDelay::decide(|count, _: &Boom| Duration::from_millis(u64::from(count)));
	assert_eq!(
		decided.clone().delay(4, &Boom("x")),
		Duration::from_millis(4)
	);
	assert!(format!("{fixed:?}").starts_with("Fixed"));
	assert!(format!("{backoff:?}").starts_with("Backoff"));
	assert_eq!(format!("{decided:?}"), "Decide(..)");
}

#[test]
fn tags_and_scope_ids() {
	let user = Tag::from("users").child(7);
	assert_eq!(user.to_string(), "users/7");
	assert_eq!(format!("{user:?}"), r#"["users", "7"]"#);
	assert_eq!(Tag::from("users".to_owned()), Tag::new(["users"]));
	assert_eq!(
		Tag::from(vec!["a".to_owned(), "b".to_owned()]),
		Tag::from(["a", "b"])
	);
	assert_eq!(user.segments(), ["users", "7"]);
	let id = ScopeId::named("todos");
	assert_eq!(id, ScopeId::named("todos"));
	assert_ne!(id.raw(), ScopeId::named("users").raw());
}

#[test]
fn options_build_merge_and_print() {
	let base: QueryOptions<Boom> = QueryOptions::new()
		.with_stale_time(Duration::from_secs(1))
		.with_gc_time(GcTime::Never)
		.with_retry(Retry::count(1))
		.with_retry_delay(Duration::from_millis(5))
		.with_retry_on_mount(false)
		.with_refetch_interval(Duration::from_secs(9))
		.with_refetch_interval_in_background(true)
		.with_refetch_on_mount(RefetchOn::Always)
		.with_refetch_on_window_focus(RefetchOn::Never)
		.with_refetch_on_reconnect(RefetchOn::Never)
		.with_network_mode(NetworkMode::OfflineFirst);
	let merged = QueryOptions::new()
		.with_stale_time(StaleTime::Static)
		.merge(&base);
	assert_eq!(
		merged.stale_time,
		Some(StaleTime::Duration(Duration::from_secs(1)))
	);
	assert_eq!(merged.network_mode, Some(NetworkMode::OfflineFirst));
	assert!(format!("{merged:?}").contains("QueryOptions"));

	let defaults = QueryDefaults::new()
		.with_stale_time(Duration::from_secs(2))
		.with_gc_time(Duration::from_secs(3))
		.with_retry(RetryPolicy::Count(4))
		.with_retry_delay(Backoff::exponential(
			Duration::from_millis(1),
			Duration::from_millis(2),
		))
		.with_retry_on_mount(false)
		.with_refetch_interval(Duration::from_secs(5))
		.with_refetch_interval_in_background(true)
		.with_refetch_on_mount(RefetchOn::Never)
		.with_refetch_on_window_focus(RefetchOn::Never)
		.with_refetch_on_reconnect(RefetchOn::Never)
		.with_network_mode(NetworkMode::Always);

	let resolved = ResolvedQueryOptions::resolve(&defaults, &QueryOptions::<Boom>::new(), false);
	assert_eq!(resolved.gc_time, GcTime::Duration(Duration::from_secs(3)));
	assert!(!resolved.retry_on_mount);
	assert!(format!("{:?}", resolved.clone()).contains("ResolvedQueryOptions"));

	let on_server =
		ResolvedQueryOptions::resolve(&QueryDefaults::new(), &QueryOptions::<Boom>::new(), true);
	assert_eq!(on_server.gc_time, GcTime::Never, "servers keep queries");
	assert!(
		!on_server.retry.should_retry(0, &Boom("x")),
		"and never retry"
	);

	assert!(!StaleTime::Static.is_stale_at(0, u64::MAX));
}

#[test]
fn filters_print_and_select() {
	let calls = Counter::new();
	let scope = doubler(calls);
	let filter = QueryFilter::key(&scope, &1).active().stale(true);
	let printed = format!("{filter:?}");
	assert!(printed.contains("QueryFilter"));
	assert!(printed.contains("Active"));
	assert_eq!(
		InvalidateOptions::default().refetch_type,
		RefetchType::Active
	);
	assert!(RefetchOptions::default().cancel_refetch);
}

#[tokio::test]
async fn filters_match_on_key_kind_and_staleness() {
	let client = quiet_client();
	let calls = Counter::new();
	// A codec, so `dehydrate_matching` can count what a filter selects.
	let scope = doubler(calls.clone()).with_ssr();
	assert_eq!(client.fetch_query(&scope, 1).await, Ok(2));
	assert_eq!(client.fetch_query(&scope, 2).await, Ok(4));

	let (owner, _observed) = owned(|| {
		use_query_in(
			client.clone(),
			scope.clone(),
			|| 1,
			QueryObserverOptions::new(),
		)
	});
	let infos = |filter: QueryFilter| {
		client
			.dehydrate_matching(filter.fetch_status(FetchStatus::Idle))
			.queries
			.len()
	};
	assert_eq!(infos(QueryFilter::key(&scope, &1)), 1);
	assert_eq!(infos(QueryFilter::scope(&scope).active()), 1);
	assert_eq!(infos(QueryFilter::scope(&scope).inactive()), 1);
	assert_eq!(infos(QueryFilter::scope(&scope).stale(false)), 2);
	assert_eq!(infos(QueryFilter::scope(&scope).stale(true)), 0);

	client.invalidate_query(&scope, &2);
	assert_eq!(infos(QueryFilter::scope(&scope).stale(true)), 1);
	client
		.invalidate_queries_with(
			QueryFilter::scope(&scope),
			InvalidateOptions {
				refetch_type: RefetchType::Inactive,
				cancel_refetch: false,
			},
		)
		.await;
	assert_eq!(calls.get(), 3, "only the unobserved query refetched");
	client
		.invalidate_queries_with(QueryFilter::scope(&scope), InvalidateOptions::REFETCH_ALL)
		.await;
	assert_eq!(calls.get(), 5);
	client.invalidate_all().await;
	assert_eq!(
		calls.get(),
		6,
		"invalidating everything refetches what is observed"
	);

	owner.cleanup();
}

#[tokio::test]
async fn the_client_from_context_and_its_defaults() {
	common::init();
	let client = QueryClient::default();
	assert!(format!("{client:?}").contains("QueryClient"));
	let calls = Counter::new();
	let scope = doubler(calls.clone());

	let (owner, (query, fetching, from_context)) = owned(|| {
		client.provide();
		// Mounting twice is a no-op.
		client.mount();
		let query = use_query(scope.clone(), || 3);
		(query, use_is_fetching(), use_query_client())
	});
	assert_eq!(from_context.fetching_count(), 1);
	assert_eq!(fetching.get_untracked(), 1);
	wait_for("loaded", || query.data().is_some()).await;
	assert_eq!(fetching.get_untracked(), 0);
	assert!(query.client().is_some());
	assert_eq!(query.info().map(|info| info.key), Some("3".to_owned()));
	assert!(format!("{query:?}").contains("Observer"));

	client.prefetch_query(&scope, 4).await;
	assert_eq!(client.get_query_data(&scope, &4), Some(8));
	assert_eq!(client.fetching_count_signal().get_untracked(), 0);

	owner.cleanup();
	client.unmount();
	assert_eq!(format!("{query:?}"), "QueryHandle(disposed)");
	assert_eq!(query.get().data, None);
	assert!(query.get().is_stale);
}

#[tokio::test]
async fn handles_expose_every_accessor() {
	let client = quiet_client();
	let calls = Counter::new();
	let scope = failing(calls.clone());

	let (owner, query) = owned(|| {
		use_query_in(
			client.clone(),
			scope.clone(),
			|| 1,
			QueryObserverOptions::new()
				.with_placeholder_fn(|previous| previous.copied().or(Some(0)))
				.with_options(QueryOptions::new().with_retry_on_mount(false))
				.with_stale_time(Duration::from_secs(1))
				.with_refetch_interval(RefetchInterval::Off)
				.with_refetch_on_mount(RefetchOn::Stale)
				.with_refetch_on_window_focus(RefetchOn::Never)
				.with_refetch_on_reconnect(RefetchOn::Never),
		)
	});
	wait_for("failed", || query.is_error()).await;
	assert_eq!(query.result(), Some(Err(Boom("down"))));
	assert!(query.error_updated_at().is_some());
	assert_eq!(
		query.data(),
		Some(0),
		"placeholder computed with no previous data"
	);
	assert!(query.is_placeholder_data());
	let selected = query.select_memo(|value| value + 1);
	assert_eq!(
		selected.get_untracked(),
		Some(1),
		"selection follows the placeholder too"
	);
	assert_eq!(query.refetch().await, Some(Err(Boom("down"))));

	owner.cleanup();
}

#[tokio::test]
async fn initial_data_with_a_timestamp_and_observer_options_print() {
	let client = quiet_client();
	let calls = Counter::new();
	let scope = doubler(calls.clone());
	let options = QueryObserverOptions::new().with_initial_data_at(40, now_ms());
	assert!(format!("{options:?}").contains("QueryObserverOptions"));
	assert_eq!(
		format!("{:?}", Placeholder::<u32>::KeepPrevious),
		"KeepPrevious"
	);
	assert_eq!(format!("{:?}", Placeholder::<u32>::None), "None");
	assert_eq!(format!("{:?}", Placeholder::Value(1_u32)), "Value(..)");

	let (owner, query) = owned(|| use_query_in(client.clone(), scope.clone(), || 20, options));
	settle().await;
	assert_eq!(
		query.data(),
		Some(40),
		"fresh initial data is not refetched"
	);
	assert_eq!(calls.get(), 0);
	owner.cleanup();
}

#[tokio::test]
async fn use_queries_tracks_a_list_of_keys() {
	let client = quiet_client();
	let calls = Counter::new();
	let scope = doubler(calls.clone());
	let keys = RwSignal::new(vec![1_u32, 2]);

	let (owner, queries) = owned(|| {
		provide_context(client.clone());
		use_queries(scope.clone(), move || keys.get())
	});
	assert_eq!(queries.len(), 2);
	assert!(!queries.is_empty());
	assert!(queries.is_pending());
	assert!(queries.is_loading());
	assert!(queries.is_fetching());
	assert_eq!(queries.all_data(), None);
	assert!(format!("{queries:?}").contains("QueriesHandle"));

	wait_for("all loaded", || queries.all_data().is_some()).await;
	assert_eq!(queries.data(), vec![Some(2), Some(4)]);
	assert_eq!(queries.all_data(), Some(vec![2, 4]));
	assert!(queries.is_success());
	assert!(!queries.is_error());
	assert!(queries.errors().is_empty());
	assert_eq!(queries.get().len(), 2);
	assert_eq!(queries.handles().len(), 2);

	queries.refetch_all();
	wait_for("refetched", || calls.get() == 4).await;

	keys.set(vec![3]);
	wait_for("rebound", || queries.data() == vec![Some(6)]).await;
	owner.cleanup();
}

#[tokio::test]
async fn mutation_scopes_keep_callbacks_across_a_new_context() {
	let client = quiet_client();
	let seen = Arc::new(AtomicU32::new(0));
	let hit = |seen: &Arc<AtomicU32>| {
		let seen = Arc::clone(seen);
		move || {
			seen.fetch_add(1, Ordering::SeqCst);
		}
	};
	let (on_success, on_error, on_settled) = (hit(&seen), hit(&seen), hit(&seen));
	let scope =
		MutationScope::new(
			|fail: bool| async move { if fail { Err(Boom("no")) } else { Ok(1_u32) } },
		)
		.with_on_success(move |_| on_success())
		.with_on_error(move |_| on_error())
		.with_on_settled(move |_| on_settled())
		.with_options(QueryOptions::new().with_network_mode(NetworkMode::Always))
		.with_on_mutate_sync(|_| 5_u8);
	assert!(format!("{scope:?}").contains("MutationScope"));
	assert!(!scope.name().is_empty());

	let mutation = use_mutation_in(client.clone(), scope.clone());
	assert!(format!("{mutation:?}").contains("MutationHandle"));
	assert!(mutation.client().is_some());
	assert!(mutation.scope().is_some());
	assert_eq!(mutation.get().status, MutationStatus::Idle);

	assert_eq!(mutation.mutate_async(false).await, Ok(1));
	assert_eq!(seen.load(Ordering::SeqCst), 2, "success and settled");
	assert_eq!(mutation.context(), Some(5));

	let failed = Arc::new(AtomicU32::new(0));
	let callbacks = {
		let failed = Arc::clone(&failed);
		MutateCallbacks::new().on_error(move |args| {
			assert_eq!(args.error, Boom("no"));
			failed.fetch_add(1, Ordering::SeqCst);
		})
	};
	assert!(format!("{callbacks:?}").contains("MutateCallbacks"));
	assert_eq!(
		mutation.mutate_with_async(true, callbacks).await,
		Err(Boom("no"))
	);
	assert_eq!(failed.load(Ordering::SeqCst), 1);
	assert_eq!(seen.load(Ordering::SeqCst), 4, "error and settled");
	let state = mutation.get();
	assert!(state.is_error());
	assert!(!state.is_idle());
}

#[tokio::test]
async fn the_mutation_cache_from_components() {
	let client = quiet_client();
	let scope = MutationScope::new(|value: u32| {
		async move {
			tokio::time::sleep(Duration::from_millis(5)).await;
			Ok::<_, Boom>(value)
		}
	})
	.with_name("from_components")
	.with_gc_time(GcTime::Never);

	let (owner, (mutation, mutating, states)) = owned(|| {
		provide_context(client.clone());
		(
			use_mutation(scope.clone()),
			use_is_mutating(),
			use_mutation_state(scope.clone()),
		)
	});
	mutation.mutate(1);
	assert_eq!(mutating.get_untracked(), 1);
	assert_eq!(client.mutating_count_signal().get_untracked(), 1);
	assert_eq!(
		client.mutating_count_matching(MutationFilter::scope(&scope)),
		1
	);
	assert_eq!(states.get_untracked().len(), 1);
	wait_for("settled", || mutating.get_untracked() == 0).await;
	assert_eq!(
		states.get_untracked().first().map(|state| state.status),
		Some(MutationStatus::Success)
	);
	client.resume_paused_mutations().await;

	client.clear();
	assert!(states.get_untracked().is_empty());
	owner.cleanup();
}

#[tokio::test]
async fn infinite_scopes_build_and_read() {
	let client = quiet_client();
	let scope = InfiniteQueryScope::new(
		|ctx: InfiniteQueryFnContext<u32, u32>| {
			let first = *ctx + ctx.page_param;
			async move { Ok::<_, Boom>(vec![first, first + 1]) }
		},
		0,
	)
	.with_next_page_param(|ctx| (ctx.pages.len() < 2).then_some(ctx.page_param + 2))
	.with_options(QueryOptions::new().with_network_mode(NetworkMode::Always))
	.with_gc_time(Duration::from_secs(60))
	.with_refetch_interval(RefetchInterval::Off)
	.with_tags(["pages"])
	.with_tags_for(|key| vec![tag!["pages", key]])
	.with_structural_sharing()
	.with_ssr();
	assert!(format!("{scope:?}").contains("InfiniteQueryScope"));
	assert_eq!(scope.id(), scope.scope().id());

	client.prefetch_infinite_query(&scope, 10).await;
	let data = client
		.fetch_infinite_query_pages(&scope, 10, 1)
		.await
		.expect("cached");
	assert_eq!(data.items().copied().collect::<Vec<_>>(), vec![10, 11]);
	assert_eq!(InfiniteData::<u32, u32>::new(), InfiniteData::default());
	assert_eq!(client.dehydrate().queries.len(), 1, "the scope has a codec");

	let (owner, pages) = owned(|| {
		provide_context(client.clone());
		use_infinite_query(scope.clone(), || 10)
	});
	assert!(format!("{pages:?}").contains("InfiniteQueryHandle"));
	assert_eq!(pages.with_pages(<[Vec<u32>]>::len), Some(1));
	assert_eq!(pages.items::<u32>(), vec![10, 11]);
	pages.fetch_next_page().await;
	pages.fetch_next_page().await;
	assert_eq!(pages.items::<u32>(), vec![10, 11, 12, 13]);
	assert!(!pages.has_next_page());
	assert_eq!(client.invalidate_tag(tag!["pages", 10]).await, ());

	owner.cleanup();
	assert!(!pages.has_next_page(), "a disposed handle has no pages");
	assert!(!pages.has_previous_page());
}

#[tokio::test]
async fn scopes_with_context_and_infallible_fetchers() {
	let client = quiet_client();
	let with_context: QueryScope<u32, bool, Boom> =
		QueryScope::new_with_context(|ctx: QueryFnContext<u32>| {
			let cancelled = ctx.signal.is_cancelled();
			let key = *ctx;
			async move { Ok(!cancelled && key == 1) }
		})
		.with_retry_delay(Duration::from_millis(1))
		.with_tags(["ctx"]);
	assert_eq!(client.fetch_query(&with_context, 1).await, Ok(true));
	assert!(format!("{with_context:?}").contains("QueryScope"));
	assert!(with_context.options().retry_delay.is_some());
	assert!(!with_context.name().is_empty());

	let infallible = QueryScope::infallible(|key: u32| async move { key + 1 });
	assert_eq!(client.ensure_query_data(&infallible, 1).await, Ok(2));
	assert_eq!(format!("{:?}", DataCodec::<u32>::json()), "DataCodec(..)");
}

#[tokio::test]
async fn subscriptions_broadcasts_and_errors_print() {
	let client = quiet_client();
	let subscription = client.subscribe(|_| {});
	assert!(format!("{subscription:?}").contains("active: true"));
	subscription.forget();
	assert_eq!(
		AlreadyInstalled.to_string(),
		"a provider was already installed"
	);
	assert!(format!("{:?}", client.invalidate_all()).contains("Refetches"));

	let disconnected = client.broadcast("nowhere");
	assert!(
		!disconnected.is_connected(),
		"no BroadcastChannel off the browser"
	);
	assert!(format!("{disconnected:?}").contains("connected: false"));
	let bus = LocalBroadcast::new();
	assert!(bus.is_empty());
	let peer = bus.peer();
	assert!(format!("{bus:?}").contains("LocalBroadcast"));
	assert!(format!("{peer:?}").contains("LocalPeer"));

	// A peer's malformed message is ignored.
	let _sync = client.broadcast_with(bus.peer());
	peer.send("not json");
	assert!(client.query_infos().is_empty());
}

#[tokio::test]
async fn managers_report_and_wait() {
	// Both managers are process-wide; this binary is the only one touching
	// them, and every client here ignores focus and reconnect.
	assert!(OnlineManager::is_online());
	assert!(FocusManager::is_focused());
	OnlineManager::set_online(true);
	OnlineManager::wait_until_online().await;
}

#[tokio::test]
async fn sleeping_without_a_runtime_uses_a_thread() {
	let elapsed = std::thread::spawn(|| {
		let start = std::time::Instant::now();
		futures::executor::block_on(sleep(Duration::from_millis(5)));
		start.elapsed()
	})
	.join()
	.expect("no panic");
	assert!(elapsed >= Duration::from_millis(5));
	sleep(Duration::ZERO).await;
}
