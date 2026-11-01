//! Retrying calls record each attempt for leptoskit's devtools.
#![cfg(feature = "devtools")]

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::MutexGuard;
use std::sync::atomic::AtomicU32;
use std::sync::atomic::Ordering;

use leptoskit_devtools_protocol::RpcAttempt;
use leptoskit_rpc::FnError;
use leptoskit_rpc::RetryPolicy;
use leptoskit_rpc::call_with_retry;
use leptoskit_rpc::devtools;

/// The timeline is process-wide, so tests that read it take turns.
static TIMELINE: Mutex<()> = Mutex::new(());

fn timeline() -> MutexGuard<'static, ()> {
	let guard = TIMELINE
		.lock()
		.unwrap_or_else(std::sync::PoisonError::into_inner);
	devtools::drain();
	guard
}

const QUICK: RetryPolicy = RetryPolicy {
	attempts: 4,
	base_delay_ms: 0,
	max_delay_ms: 0,
};

type Fetch = Pin<Box<dyn Future<Output = Result<u32, FnError>> + Send>>;

/// Fails with `errors` in turn, then answers 7.
fn flaky(errors: Vec<FnError>) -> impl Fn() -> Fetch + Send + Sync + 'static {
	let calls = Arc::new(AtomicU32::new(0));
	move || {
		let seen = calls.fetch_add(1, Ordering::SeqCst) as usize;
		let result = errors.get(seen).cloned().map_or(Ok(7), Err);
		Box::pin(async move { result })
	}
}

/// The latest record for each attempt, in the order they started.
fn latest(records: Vec<RpcAttempt>) -> Vec<RpcAttempt> {
	let mut latest: Vec<RpcAttempt> = Vec::new();

	for record in records {
		match latest.iter_mut().find(|seen| seen.id == record.id) {
			Some(seen) => *seen = record,
			None => latest.push(record),
		}
	}

	latest
}

#[test]
fn each_attempt_records_its_start_and_its_error_class() {
	let _timeline = timeline();
	let result = futures::executor::block_on(call_with_retry(
		QUICK,
		flaky(vec![
			FnError::Network("offline".to_owned()),
			FnError::Network("still offline".to_owned()),
		]),
	));
	assert_eq!(result, Ok(7));

	let records = devtools::drain();
	assert_eq!(records.len(), 6, "a start and a finish per attempt");
	let attempts = latest(records);
	assert_eq!(
		attempts
			.iter()
			.map(|attempt| (attempt.attempt, attempt.error.as_deref()))
			.collect::<Vec<_>>(),
		[(1, Some("network")), (2, Some("network")), (3, None)]
	);

	for attempt in &attempts {
		assert!(attempt.finished_ms.unwrap() >= attempt.started_ms);
		assert_eq!(attempt.function, attempts[0].function);
	}

	assert!(
		attempts[0].function.starts_with("devtools::"),
		"named after the code that made the call: {}",
		attempts[0].function
	);
	assert!(!attempts[0].function.ends_with("{{closure}}"));
}

#[test]
fn a_failure_that_is_not_retried_records_one_attempt() {
	let _timeline = timeline();
	let result = futures::executor::block_on(call_with_retry(
		QUICK,
		flaky(vec![FnError::Server("not found".to_owned())]),
	));
	assert!(result.is_err());

	let attempts = latest(devtools::drain());
	assert_eq!(attempts.len(), 1);
	assert_eq!(attempts[0].error.as_deref(), Some("server"));
}

#[test]
fn an_attempt_dropped_before_it_answers_records_a_cancellation() {
	let _timeline = timeline();
	let mut call = call_with_retry(QUICK, || -> Fetch { Box::pin(std::future::pending()) });
	let waker = futures::task::noop_waker();
	let mut context = std::task::Context::from_waker(&waker);
	assert!(call.as_mut().poll(&mut context).is_pending());

	let started = devtools::drain();
	assert_eq!(started.len(), 1);
	assert_eq!(started[0].finished_ms, None);
	drop(call);

	let finished = devtools::drain();
	assert_eq!(finished.len(), 1);
	assert_eq!(finished[0].id, started[0].id);
	assert_eq!(finished[0].error.as_deref(), Some("cancelled"));
}
