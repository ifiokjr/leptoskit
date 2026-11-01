//! A native host's plugin calls reach the Calls panel. The bridge's
//! timeline is process-wide, so this runs in its own test binary.

#![cfg(all(feature = "host", not(target_arch = "wasm32")))]

#[path = "support/hub.rs"]
mod hub;

use hub::Hub;
use leptoskit_bridge::Bridge;
use leptoskit_bridge::Bytes;
use leptoskit_bridge::Limits;
use leptoskit_bridge::MethodId;
use leptoskit_devtools::Reporter;
use leptoskit_devtools_protocol::CallRecord;
use leptoskit_devtools_protocol::CallStatus;
use leptoskit_devtools_protocol::Capability;
use leptoskit_devtools_protocol::Event;
use leptoskit_devtools_protocol::Hello;
use leptoskit_devtools_protocol::Platform;

const READ: MethodId = MethodId(3);

fn calls(state: &hub::State) -> Vec<CallRecord> {
	state
		.events
		.iter()
		.filter_map(|(_, event)| {
			match event {
				Event::Call { call } => Some(call.clone()),
				_ => None,
			}
		})
		.collect()
}

#[test]
fn plugin_calls_are_reported_without_the_devtools_plugins_own() {
	let hub = Hub::start();
	let _reporter = Reporter::start(
		&hub.url,
		Hello::new("calls", Platform::Macos, None, [Capability::Calls]),
	)
	.unwrap();
	hub.wait("a session", |state| (!state.open.is_empty()).then_some(()));

	let mut bridge = Bridge::new(Limits::default());
	let clipboard = bridge.register("clipboard", [READ]).unwrap();
	bridge.grant(clipboard, READ, true).unwrap();
	let devtools = bridge
		.register(leptoskit_plugin_devtools::NAME, [READ])
		.unwrap();
	bridge.grant(devtools, READ, true).unwrap();

	for plugin in [devtools, clipboard] {
		let id = bridge
			.submit(plugin, READ, Bytes::from_static(b"{}"))
			.unwrap();
		bridge.next_request().unwrap();
		bridge
			.complete(id, Ok(Bytes::from_static(b"\"text\"")))
			.unwrap();
		bridge.take_result(id).unwrap().unwrap().unwrap();
	}

	// The recorded call wakes the idle reporter, which sends it at once
	// rather than at its next minute-long check.
	let done = hub.wait("the clipboard call, answered", |state| {
		calls(state)
			.into_iter()
			.rfind(|call| call.plugin == "clipboard" && call.status == CallStatus::Ok)
	});
	assert!(
		done.finished_ms
			.is_some_and(|finished| finished >= done.started_ms)
	);

	// The `DevTools` plugin carries the reports themselves, so it stays off
	// the timeline.
	hub.wait("the clipboard call", |state| Some(calls(state)))
		.iter()
		.for_each(|call| assert_ne!(call.plugin, leptoskit_plugin_devtools::NAME));
}
