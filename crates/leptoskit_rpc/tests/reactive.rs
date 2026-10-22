//! The reactive layer and the wire: queries and mutations over a running
//! executor, errors that cross the server-function codec, and `FnError` as
//! a `leptoskit_query` error type.

use std::sync::Arc;
use std::sync::atomic::AtomicU32;
use std::sync::atomic::Ordering;
use std::time::Duration;

use leptos::prelude::*;
// Deprecated with `WrappedServerError`, which still exists and still needs a class.
#[allow(deprecated)]
use leptos::server_fn::error::NoCustomError;
use leptoskit_fns::FnError;
use leptoskit_fns::RetryPolicy;
use leptoskit_fns::mutation;
use leptoskit_fns::mutation_with_retry;
use leptoskit_fns::query;

/// A tokio executor for spawned work, and a root owner for this thread.
///
/// The owner matters once workspace feature unification turns on
/// `reactive_graph`'s `sandboxed-arenas`: actions and signals are then only
/// readable while an arena is active.
fn runtime() -> Owner {
	let _ = any_spawner::Executor::init_tokio();
	let owner = Owner::new();
	owner.set();
	owner
}

/// A policy that retries quickly, so tests do not wait out real backoff.
const FAST: RetryPolicy = RetryPolicy {
	attempts: 3,
	base_delay_ms: 1,
	max_delay_ms: 2,
};

/// Counts calls across clones.
#[derive(Clone, Default)]
struct Calls(Arc<AtomicU32>);

impl Calls {
	fn hit(&self) -> u32 {
		self.0.fetch_add(1, Ordering::SeqCst) + 1
	}

	fn get(&self) -> u32 {
		self.0.load(Ordering::SeqCst)
	}
}

/// Waits for `read` to return `Some`, failing after a second.
async fn eventually<T>(label: &str, read: impl Fn() -> Option<T>) -> T {
	for _ in 0..200 {
		if let Some(value) = read() {
			return value;
		}
		tokio::time::sleep(Duration::from_millis(5)).await;
	}
	panic!("timed out waiting for {label}");
}

// ---- the wire ---------------------------------------------------------------

/// Every way a server function can fail, as leptos reports it.
#[allow(deprecated)]
fn every_server_fn_error() -> Vec<(ServerFnError, FnError)> {
	vec![
		(
			ServerFnError::Request("offline".to_owned()),
			FnError::Network("offline".to_owned()),
		),
		(
			ServerFnError::Args("bad title".to_owned()),
			FnError::Invalid("bad title".to_owned()),
		),
		(
			ServerFnError::MissingArg("title".to_owned()),
			FnError::Invalid("title".to_owned()),
		),
		(
			ServerFnError::ServerError("database down".to_owned()),
			FnError::Server("database down".to_owned()),
		),
		(
			ServerFnError::MiddlewareError("rate limited".to_owned()),
			FnError::Server("rate limited".to_owned()),
		),
		(
			ServerFnError::Deserialization("bad json".to_owned()),
			FnError::Decode("bad json".to_owned()),
		),
		(
			ServerFnError::Serialization("cyclic".to_owned()),
			FnError::Decode("cyclic".to_owned()),
		),
		(
			ServerFnError::Registration("no such function".to_owned()),
			FnError::Other("no such function".to_owned()),
		),
		(
			ServerFnError::Response("truncated".to_owned()),
			FnError::Other("truncated".to_owned()),
		),
		(
			ServerFnError::WrappedServerError(NoCustomError),
			FnError::Server(NoCustomError.to_string()),
		),
	]
}

#[test]
fn every_server_fn_error_has_a_class() {
	for (error, expected) in every_server_fn_error() {
		assert_eq!(FnError::from(error), expected);
	}
}

/// The server encodes its error into the response, and the client decodes
/// it: classification has to survive the trip, or a server failure would
/// arrive looking like something else.
#[test]
fn errors_keep_their_class_across_the_wire() {
	for (error, expected) in every_server_fn_error() {
		let decoded = ServerFnError::de(error.ser());
		assert_eq!(FnError::from(decoded), expected);
	}
}

#[test]
fn errors_read_as_their_kind_and_message() {
	let cases = [
		(FnError::Network("offline".to_owned()), "network", true),
		(FnError::Invalid("bad".to_owned()), "invalid", false),
		(FnError::Server("boom".to_owned()), "server", true),
		(FnError::Decode("json".to_owned()), "decode", false),
		(FnError::Other("odd".to_owned()), "other", false),
	];
	for (error, kind, retryable) in cases {
		assert_eq!(error.retryable(), retryable, "{error}");
		assert_eq!(
			error.to_string(),
			format!("{kind} error: {}", error.message())
		);
		let boxed: Box<dyn std::error::Error> = Box::new(error.clone());
		assert_eq!(boxed.to_string(), error.to_string());
	}
}

#[test]
fn the_preset_policies() {
	assert_eq!(RetryPolicy::none().attempts, 1);
	assert_eq!(RetryPolicy::none().delay_ms(5), 0);
	assert_eq!(RetryPolicy::default(), RetryPolicy::reads());
	assert_eq!(RetryPolicy::reads().delay_ms(1), 150);
	assert_eq!(RetryPolicy::reads().delay_ms(10), 2_000, "capped");
	// A huge retry count neither overflows nor exceeds the cap.
	assert_eq!(RetryPolicy::aggressive().delay_ms(u32::MAX), 10_000);
}

// ---- queries ----------------------------------------------------------------

#[tokio::test]
async fn a_query_resolves_and_reruns_when_its_inputs_change() {
	let _owner = runtime();
	let calls = Calls::default();
	let id = RwSignal::new(1_u32);
	let todo = query(FAST, {
		let calls = calls.clone();
		move || {
			calls.hit();
			let id = id.get();
			async move { Ok::<_, FnError>(format!("todo {id}")) }
		}
	});

	assert_eq!(todo.clone().await, Ok("todo 1".to_owned()));
	id.set(2);
	let latest = eventually("the rerun", || {
		todo.get().filter(|value| value.as_deref() == Ok("todo 2"))
	})
	.await;
	assert_eq!(latest, Ok("todo 2".to_owned()));
	assert_eq!(calls.get(), 2, "one fetch per input");
}

#[tokio::test]
async fn a_query_retries_retryable_failures_and_reports_the_rest() {
	let _owner = runtime();
	let calls = Calls::default();
	let flaky = query(FAST, {
		let calls = calls.clone();
		move || {
			let attempt = calls.hit();
			async move {
				if attempt < 3 {
					Err(FnError::Network("flaky".to_owned()))
				} else {
					Ok(attempt)
				}
			}
		}
	});
	assert_eq!(flaky.await, Ok(3), "succeeds on the last attempt");

	let rejected_calls = Calls::default();
	let rejected = query(FAST, {
		let calls = rejected_calls.clone();
		move || {
			calls.hit();
			async { Err::<u32, _>(FnError::Invalid("no".to_owned())) }
		}
	});
	assert_eq!(rejected.await, Err(FnError::Invalid("no".to_owned())));
	assert_eq!(rejected_calls.get(), 1, "invalid input is never retried");
}

// ---- mutations --------------------------------------------------------------

#[tokio::test]
async fn a_mutation_runs_once_per_dispatch_and_never_retries() {
	let _owner = runtime();
	let calls = Calls::default();
	let save = mutation({
		let calls = calls.clone();
		move |title: &String| {
			calls.hit();
			let title = title.clone();
			async move {
				if title.is_empty() {
					Err(FnError::Network("dropped".to_owned()))
				} else {
					Ok(title.len())
				}
			}
		}
	});

	save.dispatch("milk".to_owned());
	assert_eq!(eventually("the result", || save.value().get()).await, Ok(4));

	save.dispatch(String::new());
	let failed = eventually("the failure", || save.value().get().filter(Result::is_err)).await;
	assert_eq!(failed, Err(FnError::Network("dropped".to_owned())));
	assert_eq!(calls.get(), 2, "a retryable failure is still not retried");
}

#[tokio::test]
async fn an_idempotent_mutation_retries_with_the_same_input() {
	let _owner = runtime();
	let calls = Calls::default();
	let seen = Arc::new(parking_lot::Mutex::new(Vec::new()));
	let put = mutation_with_retry(FAST, {
		let calls = calls.clone();
		let seen = Arc::clone(&seen);
		move |value: &u32| {
			let attempt = calls.hit();
			seen.lock().push(*value);
			let value = *value;
			async move {
				if attempt < 2 {
					Err(FnError::Server("busy".to_owned()))
				} else {
					Ok(value * 10)
				}
			}
		}
	});

	put.dispatch(7);
	assert_eq!(eventually("the result", || put.value().get()).await, Ok(70));
	assert_eq!(*seen.lock(), vec![7, 7], "the retry resent the same input");
}

// ---- leptoskit_query --------------------------------------------------------

/// `FnError` works as a `leptoskit_query` error type, and its classification
/// decides that crate's retries: a dropped connection is retried, invalid
/// input fails at once.
#[tokio::test]
async fn fn_errors_drive_leptoskit_query_retries() {
	use leptoskit_query::QueryClient;
	use leptoskit_query::QueryScope;
	use leptoskit_query::Retry;
	use leptoskit_query::RetryDelay;

	let _owner = runtime();
	let client = QueryClient::new();
	let calls = Calls::default();
	let scope: QueryScope<bool, u32, FnError> = QueryScope::new({
		let calls = calls.clone();
		move |invalid: bool| {
			let attempt = calls.hit();
			async move {
				if invalid {
					Err(FnError::Invalid("rejected".to_owned()))
				} else if attempt < 3 {
					Err(FnError::Network("offline".to_owned()))
				} else {
					Ok(attempt)
				}
			}
		}
	})
	.with_retry(Retry::decide(|failures, error: &FnError| {
		failures < 5 && error.retryable()
	}))
	.with_retry_delay(RetryDelay::Fixed(Duration::from_millis(1)));

	assert_eq!(client.fetch_query(&scope, false).await, Ok(3));
	assert_eq!(calls.get(), 3, "network failures were retried");

	assert_eq!(
		client.fetch_query(&scope, true).await,
		Err(FnError::Invalid("rejected".to_owned()))
	);
	assert_eq!(calls.get(), 4, "invalid input was not");
}
