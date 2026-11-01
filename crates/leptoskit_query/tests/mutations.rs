#![cfg(not(target_arch = "wasm32"))]

//! Mutations: lifecycle, callbacks, optimistic updates, retries, serial
//! scopes, pausing, and the mutation cache.
mod common;

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use common::Boom;
use common::Counter;
use common::owned;
use common::quiet_client;
use common::settle;
use common::wait_for;
use leptoskit_query::*;

fn todo_scope() -> QueryScope<(), Vec<String>, Boom> {
	QueryScope::new(|()| async { Ok(vec!["seed".to_owned()]) })
		.with_name("todos")
		.with_stale_time(Duration::from_secs(60))
}

fn add_todo(calls: Counter) -> MutationScope<String, String, Boom> {
	MutationScope::new(move |title: String| {
		let calls = calls.clone();
		async move {
			calls.hit();
			tokio::time::sleep(Duration::from_millis(5)).await;
			Ok(title)
		}
	})
	.with_name("add_todo")
	.with_retry_delay(Duration::from_millis(1))
}

#[tokio::test]
async fn mutate_async_reports_the_life_of_a_call() {
	let client = quiet_client();
	let calls = Counter::new();
	let add = use_mutation_in(client.clone(), add_todo(calls.clone()));

	assert!(add.is_idle());
	assert_eq!(add.submitted_at(), None);
	assert_eq!(add.data(), None);

	let outcome = add.mutate_async("Buy milk".to_owned()).await;
	assert_eq!(outcome, Ok("Buy milk".to_owned()));
	assert_eq!(calls.get(), 1);
	assert!(add.is_success());
	assert!(!add.is_pending());
	assert_eq!(add.data(), Some("Buy milk".to_owned()));
	assert_eq!(add.variables(), Some("Buy milk".to_owned()));
	assert!(add.submitted_at().is_some());

	assert_eq!(add.context(), Some(()), "the default context is unit");
	assert_eq!(client.mutating_count(), 0);

	add.mutate("Fire and forget".to_owned());
	assert!(add.is_pending());
	wait_for("settled", || {
		add.data() == Some("Fire and forget".to_owned())
	})
	.await;

	add.reset();
	assert!(add.is_idle());
	assert_eq!(add.data(), None);
}

#[tokio::test]
async fn errors_surface_after_retries() {
	let client = quiet_client();
	let calls = Counter::new();
	let scope: MutationScope<u32, u32, Boom> = {
		let calls = calls.clone();
		MutationScope::new(move |attempt: u32| {
			let calls = calls.clone();
			async move {
				calls.hit();
				let _ = attempt;
				Err(Boom("rejected"))
			}
		})
	}
	.with_retry(Retry::count(2))
	.with_retry_delay(Duration::from_millis(1));

	let fail = use_mutation_in(client.clone(), scope);
	assert_eq!(fail.mutate_async(1).await, Err(Boom("rejected")));
	assert_eq!(calls.get(), 3, "one attempt plus two retries");
	assert!(fail.is_error());
	assert_eq!(fail.error(), Some(Boom("rejected")));
	assert_eq!(fail.failure_count(), 3);
	assert_eq!(fail.failure_reason(), Some(Boom("rejected")));
}

#[tokio::test]
async fn on_mutate_context_flows_to_every_callback() {
	let client = quiet_client();
	type Log = Arc<parking_lot::Mutex<Vec<&'static str>>>;
	let log = |log: &Log, label: &'static str| {
		let log = Arc::clone(log);
		move || log.lock().push(label)
	};

	let seen: Log = Arc::default();
	let scope = {
		let on_mutate = log(&seen, "mutate");
		let on_success = log(&seen, "success");
		let on_settled = log(&seen, "settled");
		MutationScope::<u32, u32, Boom>::new(|value: u32| async move { Ok(value * 10) })
			.with_on_mutate_sync(move |args| {
				on_mutate();
				args.variables * 2
			})
			.with_on_success(move |args| {
				on_success();
				assert_eq!(args.context, 84);
				assert_eq!(args.variables, 42);
			})
			.with_on_error(|_| unreachable!("this call succeeds"))
			.with_on_settled(move |args| {
				on_settled();
				assert_eq!(args.result, Ok(420));
			})
	};
	let succeed = use_mutation_in(client.clone(), scope.with_name("ok"));
	assert_eq!(succeed.mutate_async(42).await, Ok(420));
	assert_eq!(*seen.lock(), vec!["mutate", "success", "settled"]);

	let seen_error: Log = Arc::default();
	let scope = {
		let on_mutate = log(&seen_error, "mutate");
		let on_error = log(&seen_error, "error");
		let on_settled = log(&seen_error, "settled");
		MutationScope::<u32, u32, Boom>::new(|value: u32| {
			async move {
				let _ = value;
				Err(Boom("no"))
			}
		})
		.with_retry(Retry::never())
		.with_on_mutate_sync(move |args| {
			on_mutate();
			args.variables
		})
		.with_on_error(move |args| {
			on_error();
			assert_eq!(args.context, 7);
		})
		.with_on_settled(move |args| {
			on_settled();
			assert_eq!(args.result, Err(Boom("no")));
		})
	};
	let fail = use_mutation_in(client.clone(), scope.with_name("fail"));
	assert_eq!(fail.mutate_async(7).await, Err(Boom("no")));
	assert_eq!(*seen_error.lock(), vec!["mutate", "error", "settled"]);
}

#[tokio::test]
async fn optimistic_updates_roll_back_on_error() {
	let client = quiet_client();
	let todos = todo_scope();
	assert_eq!(
		client.fetch_query(&todos, ()).await,
		Ok(vec!["seed".to_owned()])
	);

	let scope: MutationScope<String, String, Boom, Option<Vec<String>>> = {
		let todos = todos.clone();
		MutationScope::new(|title: String| {
			async move {
				let _ = title;
				Err(Boom("offline"))
			}
		})
		.with_retry(Retry::never())
		.with_on_mutate_sync({
			let todos = todos.clone();
			move |args| {
				let previous = args.client.get_query_data(&todos, &());
				args.client.update_query_data(&todos, (), |list| {
					let mut list = list.unwrap_or_default();
					list.push(args.variables.clone());
					Some(list)
				});
				previous
			}
		})
		.with_on_error({
			let todos = todos.clone();
			move |args| {
				if let Some(previous) = args.context {
					args.client.set_query_data(&todos, (), previous);
				}
			}
		})
	};
	let add = use_mutation_in(client.clone(), scope);

	assert_eq!(
		add.mutate_async("Optimistic".to_owned()).await,
		Err(Boom("offline"))
	);
	assert_eq!(
		client.get_query_data(&todos, &()),
		Some(vec!["seed".to_owned()]),
		"the optimistic write was rolled back"
	);
}

#[tokio::test]
async fn awaited_settled_keeps_the_mutation_pending_until_refetches_finish() {
	let client = quiet_client();
	let fetches = Counter::new();
	let todos: QueryScope<(), Vec<String>, Boom> = {
		let fetches = fetches.clone();
		QueryScope::new(move |()| {
			let fetches = fetches.clone();
			async move {
				let seen = fetches.hit();
				Ok(vec![format!("v{seen}")])
			}
		})
		.with_name("versioned")
		.with_stale_time(Duration::from_secs(60))
	};
	// Invalidation only refetches observed queries, so observe it.
	let (owner, observed) = owned(|| {
		use_query_in(
			client.clone(),
			todos.clone(),
			|| (),
			QueryObserverOptions::new(),
		)
	});
	wait_for("observed", || {
		observed.data() == Some(vec!["v1".to_owned()])
	})
	.await;

	// The refetch takes a moment, so an awaited `on_settled` holds the
	// mutation in `pending` until it lands — TanStack's behavior.
	let gate = Arc::new(std::sync::atomic::AtomicBool::new(false));
	let scope: MutationScope<(), (), Boom> = {
		let todos = todos.clone();
		let gate = Arc::clone(&gate);
		MutationScope::new(|()| async { Ok(()) })
			.with_on_settled_async(move |args| {
				let todos = todos.clone();
				let gate = Arc::clone(&gate);
				async move {
					while !gate.load(Ordering::SeqCst) {
						tokio::time::sleep(Duration::from_millis(2)).await;
					}

					args.client.invalidate_scope(&todos).await;
				}
			})
			.with_name("touch")
	};
	let touch = use_mutation_in(client.clone(), scope);

	touch.mutate(());
	settle().await;
	assert!(
		touch.is_pending(),
		"the mutation has not settled: its on_settled is still awaiting"
	);

	gate.store(true, Ordering::SeqCst);
	wait_for("settled", || !touch.is_pending()).await;
	assert_eq!(fetches.get(), 2, "the call settled only after the refetch");
	assert_eq!(observed.data(), Some(vec!["v2".to_owned()]));
	owner.cleanup();
}

#[tokio::test]
async fn serial_scopes_run_one_at_a_time_in_order() {
	let client = quiet_client();
	let log: Arc<parking_lot::Mutex<Vec<u32>>> = Arc::default();

	let scope: MutationScope<u32, u32, Boom> = {
		let log = Arc::clone(&log);
		MutationScope::new(move |id: u32| {
			let log = Arc::clone(&log);
			async move {
				log.lock().push(id);
				tokio::time::sleep(Duration::from_millis(30)).await;
				log.lock().push(id);
				Ok(id)
			}
		})
		.with_serial_scope("uploads")
	};
	let first = use_mutation_in(client.clone(), scope.clone());
	let second = use_mutation_in(client.clone(), scope.with_name("second"));

	let a = first.mutate_async(1);
	let b = second.mutate_async(2);
	let (a, b) = futures::join!(a, b);
	assert_eq!((a, b), (Ok(1), Ok(2)));
	assert_eq!(
		*log.lock(),
		vec![1, 1, 2, 2],
		"the second waited for the first"
	);
}

#[tokio::test]
async fn paused_mutations_resume_when_the_network_returns() {
	common::init();
	let client = QueryClient::new().with_defaults(
		QueryDefaults::new()
			.with_refetch_on_window_focus(RefetchOn::Never)
			.with_refetch_on_reconnect(RefetchOn::Never),
	);
	let calls = Counter::new();
	let scope = add_todo(calls.clone()).with_network_mode(NetworkMode::Online);
	let add = use_mutation_in(client.clone(), scope);

	OnlineManager::set_online(false);
	let outcome = add.mutate_async("queued".to_owned());
	settle().await;
	assert!(add.is_paused());
	assert!(add.is_pending());
	assert_eq!(calls.get(), 0);

	OnlineManager::set_online(true);
	assert_eq!(outcome.await, Ok("queued".to_owned()));
	assert!(add.is_success());
}

#[tokio::test]
async fn mutation_state_lists_cached_mutations() {
	let client = quiet_client();
	let slow = Arc::new(std::sync::atomic::AtomicBool::new(false));
	let scope: MutationScope<String, String, Boom> = {
		let slow = Arc::clone(&slow);
		MutationScope::new(move |title: String| {
			let slow = Arc::clone(&slow);
			async move {
				while !slow.load(Ordering::SeqCst) {
					tokio::time::sleep(Duration::from_millis(2)).await;
				}

				Ok(title)
			}
		})
	}
	.with_name("slow")
	.with_gc_time(Duration::from_secs(60));

	let add = use_mutation_in(client.clone(), scope.clone());
	add.mutate("one".to_owned());
	add.mutate("two".to_owned());

	wait_for("both visible", || {
		client.mutation_states(&scope).len() == 2
			&& client
				.mutation_states(&scope)
				.iter()
				.all(MutationState::is_pending)
	})
	.await;
	assert_eq!(client.mutating_count(), 2);
	assert_eq!(
		client
			.mutation_infos(MutationFilter::scope(&scope).pending())
			.len(),
		2
	);

	slow.store(true, Ordering::SeqCst);
	wait_for("settled", || add.data().is_some()).await;
	let states = client.mutation_states(&scope);
	assert_eq!(states.len(), 2);
	assert!(states.iter().all(MutationState::is_success));
}

#[tokio::test]
async fn per_call_callbacks_fire_for_that_call_only() {
	let client = quiet_client();
	let calls = Counter::new();
	let scope = add_todo(calls.clone());
	let add = use_mutation_in(client.clone(), scope);

	let seen: Arc<parking_lot::Mutex<Vec<&'static str>>> = Arc::default();
	let first = {
		let seen = Arc::clone(&seen);
		MutateCallbacks::new().on_success(move |_| seen.lock().push("first"))
	};
	add.mutate_with("a".to_owned(), first);

	let second = {
		let seen = Arc::clone(&seen);
		let settled_seen = Arc::clone(&seen);
		MutateCallbacks::<String, String, Boom, ()>::new()
			.on_success(move |_| seen.lock().push("second"))
			.on_settled(move |_| settled_seen.lock().push("second settled"))
	};
	add.mutate_with("b".to_owned(), second);

	wait_for("both settled", || seen.lock().len() >= 3).await;
	assert_eq!(*seen.lock(), vec!["first", "second", "second settled"]);
}

#[tokio::test]
async fn settled_mutations_are_garbage_collected() {
	let client = quiet_client();
	let scope = add_todo(Counter::new()).with_gc_time(Duration::from_millis(20));
	let add = use_mutation_in(client.clone(), scope.clone());

	assert_eq!(
		add.mutate_async("gone".to_owned()).await,
		Ok("gone".to_owned())
	);
	assert_eq!(client.mutation_infos(MutationFilter::all()).len(), 1);
	wait_for("collected", || {
		client.mutation_infos(MutationFilter::all()).is_empty()
	})
	.await;
	assert_eq!(
		add.data(),
		Some("gone".to_owned()),
		"the handle keeps its view"
	);
}

#[tokio::test]
async fn mutation_cache_events_are_observable() {
	let client = quiet_client();
	let added = Counter::new();
	let subscription = {
		let added = added.clone();
		client.subscribe_mutations(move |notification| {
			if notification.event == MutationEvent::Added {
				added.hit();
			}
		})
	};
	let add = use_mutation_in(client.clone(), add_todo(Counter::new()));
	let _ = add.mutate_async("x".to_owned()).await;
	assert_eq!(added.get(), 1);
	drop(subscription);
}
