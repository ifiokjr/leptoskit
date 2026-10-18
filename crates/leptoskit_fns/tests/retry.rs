//! Retry policy and error classification tests.

use std::sync::Arc;
use std::sync::atomic::AtomicU32;
use std::sync::atomic::Ordering;

use leptos::prelude::ServerFnError;
use leptoskit_fns::FnError;
use leptoskit_fns::RetryPolicy;
use leptoskit_fns::call_with_retry;

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
	assert!(FnError::from(ServerFnError::ServerError("boom".to_owned())).retryable());
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
