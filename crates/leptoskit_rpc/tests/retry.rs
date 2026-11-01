//! Retry policy and error classification tests.

use std::sync::Arc;
use std::sync::atomic::AtomicU32;
use std::sync::atomic::Ordering;

use leptos::prelude::ServerFnError;
use leptoskit_rpc::FnError;
use leptoskit_rpc::RetryPolicy;
use leptoskit_rpc::call_with_retry;

#[test]
fn delays_double_up_to_the_cap() {
	let policy = RetryPolicy::aggressive();
	assert_eq!(policy.delay_ms(0), 0);
	assert_eq!(policy.delay_ms(1), 250);
	assert_eq!(policy.delay_ms(2), 500);
	assert_eq!(policy.delay_ms(3), 1_000);
	assert_eq!(policy.delay_ms(4), 2_000, "capped at max_delay_ms");
}

#[test]
fn errors_classify_by_kind() {
	assert!(FnError::from(ServerFnError::Request("offline".to_owned())).retryable());
	assert!(
		!FnError::from(ServerFnError::ServerError("not found".to_owned())).retryable(),
		"a failure the function reported, deliberate or not, is not retried"
	);
	assert!(!FnError::from(ServerFnError::MissingArg("title".to_owned())).retryable());
	assert!(!FnError::from(ServerFnError::Deserialization("bad json".to_owned())).retryable());
	assert_eq!(
		FnError::from(ServerFnError::Request("offline".to_owned())).message(),
		"offline"
	);
}

#[tokio::test]
async fn retries_until_success() {
	let calls = Arc::new(AtomicU32::new(0));
	let calls_for_retry = Arc::clone(&calls);
	let result: Result<u32, FnError> = call_with_retry(
		RetryPolicy {
			attempts: 5,
			base_delay_ms: 1,
			max_delay_ms: 2,
		},
		move || {
			let calls = Arc::clone(&calls_for_retry);
			Box::pin(async move {
				let seen = calls.fetch_add(1, Ordering::SeqCst);
				if seen < 3 {
					Err(FnError::Network("flaky".to_owned()))
				} else {
					Ok(42)
				}
			})
		},
	)
	.await;

	assert_eq!(result.expect("succeeds on the fourth attempt"), 42);
	assert_eq!(calls.load(Ordering::SeqCst), 4);
}

#[tokio::test]
async fn non_retryable_errors_fail_fast() {
	let calls = Arc::new(AtomicU32::new(0));
	let calls_for_retry = Arc::clone(&calls);
	let result: Result<u32, FnError> = call_with_retry(RetryPolicy::aggressive(), move || {
		let calls = Arc::clone(&calls_for_retry);
		Box::pin(async move {
			calls.fetch_add(1, Ordering::SeqCst);
			Err(FnError::Invalid("validation".to_owned()))
		})
	})
	.await;

	assert!(matches!(result, Err(FnError::Invalid(_))));
	assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn exhausts_attempts() {
	let calls = Arc::new(AtomicU32::new(0));
	let calls_for_retry = Arc::clone(&calls);
	let result: Result<u32, FnError> = call_with_retry(
		RetryPolicy {
			attempts: 3,
			base_delay_ms: 1,
			max_delay_ms: 1,
		},
		move || {
			let calls = Arc::clone(&calls_for_retry);
			Box::pin(async move {
				calls.fetch_add(1, Ordering::SeqCst);
				Err(FnError::Network("down".to_owned()))
			})
		},
	)
	.await;

	assert!(matches!(result, Err(FnError::Network(_))));
	assert_eq!(calls.load(Ordering::SeqCst), 3);
}

/// Backoff needs no tokio runtime: a plain executor waits it out too, as a
/// native shell or a test without `#[tokio::test]` would.
#[test]
fn retries_back_off_without_a_tokio_runtime() {
	assert!(
		tokio::runtime::Handle::try_current().is_err(),
		"this test must run outside any runtime"
	);
	let calls = Arc::new(AtomicU32::new(0));
	let calls_for_retry = Arc::clone(&calls);
	let started = std::time::Instant::now();
	let result: Result<u32, FnError> = futures::executor::block_on(call_with_retry(
		RetryPolicy {
			attempts: 3,
			base_delay_ms: 5,
			max_delay_ms: 5,
		},
		move || {
			let calls = Arc::clone(&calls_for_retry);
			Box::pin(async move {
				if calls.fetch_add(1, Ordering::SeqCst) < 2 {
					Err(FnError::Network("flaky".to_owned()))
				} else {
					Ok(7)
				}
			})
		},
	));
	assert_eq!(result, Ok(7));
	assert_eq!(calls.load(Ordering::SeqCst), 3);
	assert!(
		started.elapsed() >= std::time::Duration::from_millis(10),
		"both backoffs were waited out"
	);
}

#[tokio::test]
async fn server_failures_are_not_retried() {
	let calls = Arc::new(AtomicU32::new(0));
	let calls_for_retry = Arc::clone(&calls);
	let result: Result<u32, FnError> = call_with_retry(RetryPolicy::aggressive(), move || {
		let calls = Arc::clone(&calls_for_retry);
		Box::pin(async move {
			calls.fetch_add(1, Ordering::SeqCst);
			Err(FnError::from(ServerFnError::new("not found")))
		})
	})
	.await;

	assert!(matches!(result, Err(FnError::Server(_))));
	assert_eq!(calls.load(Ordering::SeqCst), 1, "a refusal is final");
}

#[test]
fn wire_text_classifies_like_the_server() {
	assert_eq!(
		FnError::from_wire("Request|offline"),
		FnError::Network("offline".to_owned())
	);
	assert_eq!(
		FnError::from_wire("MissingArg|title"),
		FnError::Invalid("title".to_owned())
	);
	assert_eq!(
		FnError::from_wire("WrappedServerFn|boom"),
		FnError::Server("boom".to_owned())
	);
	assert_eq!(
		FnError::from_wire("Serialization|bad"),
		FnError::Decode("bad".to_owned())
	);
	assert_eq!(
		FnError::from_wire("Registration|unknown function"),
		FnError::Other("unknown function".to_owned())
	);
	assert_eq!(
		FnError::from_wire("Mystery|text"),
		FnError::Other("Mystery|text".to_owned()),
		"unknown kinds keep the whole text"
	);
	assert_eq!(
		FnError::from_wire("Args|b|c").message(),
		"b|c",
		"only the first bar splits"
	);
	assert_eq!(
		FnError::from_wire("no separator"),
		FnError::Other("no separator".to_owned())
	);

	assert_eq!(FnError::from_wire("Args|bad input").message(), "bad input");
	assert_eq!(FnError::from_wire("ServerError|down").message(), "down");
	assert_eq!(FnError::from_wire("Deserialization|odd").message(), "odd");
	assert_eq!(
		FnError::from_wire("Request|offline").to_string(),
		"network error: offline"
	);
}
