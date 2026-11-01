//! The calls timeline records each step of a call, for leptoskit's devtools.
#![cfg(feature = "devtools")]

use std::sync::Mutex;
use std::sync::MutexGuard;

use leptoskit_bridge::Bridge;
use leptoskit_bridge::Bytes;
use leptoskit_bridge::Error;
use leptoskit_bridge::Limits;
use leptoskit_bridge::MethodId;
use leptoskit_bridge::PluginId;
use leptoskit_bridge::devtools;
use leptoskit_devtools_protocol::CallRecord;
use leptoskit_devtools_protocol::CallStatus;
use leptoskit_devtools_protocol::sink::CAPACITY;

const READ: MethodId = MethodId(3);

/// The timeline is process-wide, so tests that read it take turns.
static TIMELINE: Mutex<()> = Mutex::new(());

fn timeline() -> MutexGuard<'static, ()> {
	let guard = TIMELINE
		.lock()
		.unwrap_or_else(std::sync::PoisonError::into_inner);
	devtools::drain();
	guard
}

fn bridge(name: &str) -> (Bridge, PluginId) {
	let mut bridge = Bridge::new(Limits::default());
	let plugin = bridge.register(name, [READ]).unwrap();
	bridge.grant(plugin, READ, true).unwrap();
	(bridge, plugin)
}

fn records(plugin: &str) -> Vec<CallRecord> {
	devtools::drain()
		.into_iter()
		.filter(|record| record.plugin == plugin)
		.collect()
}

#[test]
fn a_call_records_submit_run_answer_and_read() {
	let _timeline = timeline();
	let (mut bridge, plugin) = bridge("timeline.ok");
	let id = bridge
		.submit(plugin, READ, Bytes::from_static(b"args"))
		.unwrap();
	let request = bridge.next_request().unwrap();
	bridge
		.complete(id, Ok(Bytes::from_static(b"the answer")))
		.unwrap();
	assert_eq!(request.payload.len(), 4);
	bridge.take_result(id).unwrap().unwrap().unwrap();

	let records = records("timeline.ok");
	assert_eq!(records.len(), 4, "submit, run, answer, read: {records:#?}");
	assert!(records.iter().all(|record| record.id == records[0].id));
	assert!(
		records
			.iter()
			.all(|record| record.method == "3" && record.request_bytes == Some(4))
	);

	let [submitted, running, answered, taken] = &records[..] else {
		unreachable!()
	};
	assert_eq!(submitted.status, CallStatus::Pending);
	assert_eq!(submitted.running_ms, None);
	assert_eq!(running.status, CallStatus::Pending);
	assert!(running.running_ms.unwrap() >= running.started_ms);
	assert_eq!(answered.status, CallStatus::Ok);
	assert_eq!(answered.response_bytes, Some(10));

	assert!(answered.finished_ms.unwrap() >= answered.running_ms.unwrap());
	assert_eq!(answered.taken_ms, None);
	assert!(taken.taken_ms.unwrap() >= taken.finished_ms.unwrap());
	assert_eq!(taken.status, CallStatus::Ok);
	assert_eq!(taken.completed_late_ms, None);
}

#[test]
fn a_failed_call_records_an_error_without_an_answer_size() {
	let _timeline = timeline();
	let (mut bridge, plugin) = bridge("timeline.error");
	let id = bridge.submit(plugin, READ, Bytes::new()).unwrap();
	bridge.next_request().unwrap();
	bridge.complete(id, Err(Error::Native)).unwrap();

	let last = records("timeline.error").pop().unwrap();
	assert_eq!(last.status, CallStatus::Error);
	assert_eq!(last.response_bytes, None);
	assert_eq!(last.request_bytes, Some(0));
	assert!(last.finished_ms.is_some());
}

#[test]
fn an_oversized_answer_records_the_error_it_became() {
	let _timeline = timeline();
	let mut bridge = Bridge::new(Limits {
		payload_bytes: 4,
		..Limits::default()
	});
	let plugin = bridge.register("timeline.too_large", [READ]).unwrap();
	bridge.grant(plugin, READ, true).unwrap();
	let id = bridge.submit(plugin, READ, Bytes::new()).unwrap();
	bridge.next_request().unwrap();
	bridge
		.complete(id, Ok(Bytes::from_static(b"far too long")))
		.unwrap();

	let last = records("timeline.too_large").pop().unwrap();
	assert_eq!(last.status, CallStatus::Error);
	assert_eq!(last.response_bytes, None);
}

#[test]
fn cancelling_a_queued_call_records_it_without_a_run() {
	let _timeline = timeline();
	let (mut bridge, plugin) = bridge("timeline.cancel_queued");
	let id = bridge.submit(plugin, READ, Bytes::new()).unwrap();
	bridge.cancel(id).unwrap();

	let records = records("timeline.cancel_queued");
	assert_eq!(records.len(), 2);
	assert_eq!(records[1].status, CallStatus::Cancelled);
	assert_eq!(records[1].running_ms, None);
	assert!(records[1].finished_ms.is_some());
}

#[test]
fn cancelling_a_ready_call_keeps_when_it_answered() {
	let _timeline = timeline();
	let (mut bridge, plugin) = bridge("timeline.cancel_ready");
	let id = bridge.submit(plugin, READ, Bytes::new()).unwrap();
	bridge.next_request().unwrap();
	bridge.complete(id, Ok(Bytes::from_static(b"ok"))).unwrap();
	bridge.cancel(id).unwrap();

	let records = records("timeline.cancel_ready");
	let [.., answered, cancelled] = &records[..] else {
		unreachable!()
	};
	assert_eq!(cancelled.status, CallStatus::Cancelled);
	assert_eq!(cancelled.finished_ms, answered.finished_ms);
	assert_eq!(cancelled.response_bytes, None, "the answer was never read");
}

#[test]
fn an_answer_after_cancelling_records_a_late_completion() {
	let _timeline = timeline();
	let (mut bridge, plugin) = bridge("timeline.late");

	// Answered before the host took the cancellation.
	let early = bridge.submit(plugin, READ, Bytes::new()).unwrap();
	// Answered after the host took the cancellation.
	let late = bridge.submit(plugin, READ, Bytes::new()).unwrap();
	bridge.next_request().unwrap();
	bridge.next_request().unwrap();
	bridge.cancel(early).unwrap();
	bridge.cancel(late).unwrap();
	assert_eq!(
		bridge.complete(early, Ok(Bytes::new())),
		Err(Error::InvalidCall)
	);
	assert_eq!(bridge.next_cancelled(), Some(early));

	assert_eq!(bridge.next_cancelled(), Some(late));
	assert_eq!(
		bridge.complete(late, Ok(Bytes::new())),
		Err(Error::InvalidCall)
	);
	// A second answer records nothing more.
	assert_eq!(
		bridge.complete(late, Ok(Bytes::new())),
		Err(Error::InvalidCall)
	);

	let records = records("timeline.late");
	let lates: Vec<_> = records
		.iter()
		.filter(|record| record.completed_late_ms.is_some())
		.collect();
	assert_eq!(lates.len(), 2, "{records:#?}");
	assert_ne!(lates[0].id, lates[1].id);

	for record in lates {
		assert_eq!(record.status, CallStatus::Cancelled);
		assert!(record.completed_late_ms >= record.finished_ms);
	}
}

#[test]
fn an_answer_to_an_unknown_call_records_nothing() {
	let _timeline = timeline();
	let (mut bridge, plugin) = bridge("timeline.unknown");
	let id = bridge.submit(plugin, READ, Bytes::new()).unwrap();
	bridge.next_request().unwrap();
	bridge.complete(id, Ok(Bytes::new())).unwrap();
	bridge.take_result(id).unwrap();
	assert_eq!(
		bridge.complete(id, Ok(Bytes::new())),
		Err(Error::InvalidCall)
	);

	let records = records("timeline.unknown");
	assert_eq!(records.len(), 4);
	assert!(
		records
			.iter()
			.all(|record| record.completed_late_ms.is_none())
	);
}

#[test]
fn a_full_timeline_drops_the_oldest_records_and_counts_them() {
	let _timeline = timeline();
	let (mut bridge, plugin) = bridge("timeline.overflow");
	let dropped = devtools::dropped();
	let calls = CAPACITY + 10;

	for _ in 0..calls {
		let id = bridge.submit(plugin, READ, Bytes::new()).unwrap();
		bridge.cancel(id).unwrap();
	}

	let records = devtools::drain();
	assert_eq!(records.len(), CAPACITY);
	assert_eq!(devtools::dropped() - dropped, (calls * 2 - CAPACITY) as u64);
	let last = records.last().unwrap();
	assert_eq!(last.plugin, "timeline.overflow");
	assert_eq!(last.status, CallStatus::Cancelled);
	assert!(
		records.windows(2).all(|pair| pair[0].id <= pair[1].id),
		"oldest first"
	);
}
