use std::collections::VecDeque;

use leptoskit_devtools_protocol::Capability;
use leptoskit_devtools_protocol::Command;
use leptoskit_devtools_protocol::Event;
use leptoskit_devtools_protocol::MAX_VALUE_BYTES;
use leptoskit_devtools_protocol::MatchKey;
use leptoskit_devtools_protocol::PROTOCOL_VERSION;
use leptoskit_devtools_protocol::Value;
use serde_json::json;

use super::*;

fn state(name: &str, bytes: usize) -> Event {
	Event::StateChanged {
		name: name.into(),
		value: Value::from_json("String", json!("x".repeat(bytes))),
	}
}

#[test]
fn a_hello_round_trips_and_names_its_protocol() {
	let hello = Hello::new([Capability::State, Capability::Navigation]);
	assert_eq!(hello.protocol, PROTOCOL_VERSION);
	assert_eq!(Hello::decode(&hello.encode().unwrap()), Ok(hello));

	let other =
		serde_json::to_vec(&json!({ "protocol": PROTOCOL_VERSION + 1, "capabilities": [] }))
			.unwrap();
	assert_eq!(Hello::decode(&other), Err(Error::InvalidArgument));
	assert_eq!(Hello::decode(b"{"), Err(Error::InvalidArgument));
	assert_eq!(
		Hello::decode(&vec![b' '; MAX_HELLO_BYTES + 1]),
		Err(Error::TooLarge)
	);
}

#[test]
fn a_report_round_trips_in_one_payload() {
	let report = Report {
		reset: true,
		events: vec![
			Event::RouteChanged {
				route: "/clips".into(),
			},
			state("store", 10),
		],
	};
	let payloads = report.encode().unwrap();
	assert_eq!(payloads.len(), 1);
	assert_eq!(Report::decode(&payloads[0]), Ok(report));

	// An empty update is still one payload, which the host acknowledges.
	let payloads = Report::default().encode().unwrap();
	assert_eq!(payloads, [br#"{"events":[]}"#.to_vec()]);
	assert_eq!(Report::decode(&payloads[0]), Ok(Report::default()));
}

#[test]
fn a_large_report_splits_and_resets_only_in_its_first_payload() {
	// Each value is just under the protocol's limit, so twenty of them need
	// two payloads.
	let events: Vec<Event> = (0..20)
		.map(|index| state(&format!("value-{index}"), MAX_VALUE_BYTES - 64))
		.collect();
	let report = Report {
		reset: true,
		events: events.clone(),
	};
	let payloads = report.encode().unwrap();
	assert_eq!(payloads.len(), 2);
	let decoded: Vec<Report> = payloads
		.iter()
		.map(|payload| {
			assert!(payload.len() <= MAX_REPORT_BYTES);
			Report::decode(payload).unwrap()
		})
		.collect();
	assert!(decoded[0].reset);
	assert!(!decoded[1].reset);
	let joined: Vec<Event> = decoded
		.into_iter()
		.flat_map(|report| report.events)
		.collect();
	assert_eq!(joined, events);
}

#[test]
fn a_report_holds_only_values() {
	let report = Report {
		reset: false,
		events: vec![Event::Picked {
			node: leptoskit_devtools_protocol::NodeRef::Native(1),
			key: MatchKey::new("key"),
		}],
	};
	assert_eq!(report.encode(), Err(Error::InvalidArgument));
	let forged = serde_json::to_vec(&report).unwrap();
	assert_eq!(Report::decode(&forged), Err(Error::InvalidArgument));
	assert_eq!(
		Report::decode(&vec![b' '; MAX_REPORT_BYTES + 1]),
		Err(Error::TooLarge)
	);
}

#[test]
fn a_command_reply_is_bounded_and_keeps_only_the_guests_commands() {
	let mut queue: VecDeque<Command> = (0..MAX_COMMANDS + 3)
		.map(|index| {
			Command::Navigate {
				href: format!("/clips/{index}"),
			}
		})
		.collect();
	let reply = encode_commands(&mut queue);
	assert_eq!(decode_commands(&reply).unwrap().len(), MAX_COMMANDS);
	assert_eq!(queue.len(), 3);

	let reply = serde_json::to_vec(&[
		Command::StartProfiling,
		Command::SetState {
			name: "store".into(),
			value: Value::from_json("usize", json!(2)),
		},
	])
	.unwrap();
	assert_eq!(
		decode_commands(&reply).unwrap(),
		[Command::SetState {
			name: "store".into(),
			value: Value::from_json("usize", json!(2)),
		}]
	);
	assert_eq!(decode_commands(b"{}"), Err(Error::InvalidArgument));
}

#[test]
fn a_wait_is_bounded() {
	assert_eq!(decode_wait(&encode_wait(100)), Ok(100));
	assert_eq!(decode_wait(&encode_wait(60_000)), Ok(MAX_WAIT_MS));
	assert_eq!(
		decode_wait(&(MAX_WAIT_MS + 1).to_le_bytes()),
		Err(Error::InvalidArgument)
	);
	assert_eq!(decode_wait(&[0; 3]), Err(Error::InvalidArgument));
}

#[cfg(all(feature = "async-host", not(target_arch = "wasm32")))]
mod host {
	use std::collections::BTreeSet;
	use std::sync::Arc;
	use std::sync::Mutex;
	use std::time::Duration;

	use leptoskit_bridge::Bytes;
	use leptoskit_bridge::async_host::AsyncPlugin;
	use leptoskit_bridge::async_host::CancellationToken;

	use super::*;

	#[derive(Clone, Default)]
	struct Recorder {
		hellos: Arc<Mutex<Vec<BTreeSet<Capability>>>>,
		reports: Arc<Mutex<Vec<Report>>>,
	}

	impl Link for Recorder {
		fn hello(&self, capabilities: BTreeSet<Capability>) {
			self.hellos.lock().unwrap().push(capabilities);
		}

		fn report(&self, report: Report) {
			self.reports.lock().unwrap().push(report);
		}
	}

	fn set_state(count: usize) -> Command {
		Command::SetState {
			name: "count".into(),
			value: Value::from_json("usize", json!(count)),
		}
	}

	#[tokio::test]
	async fn hellos_and_reports_reach_the_link() {
		let recorder = Recorder::default();
		let plugin = DevtoolsPlugin::new(recorder.clone(), Inbox::new());
		let hello = Hello::new([Capability::State]);
		let reply = plugin
			.invoke(
				HELLO,
				Bytes::from(hello.encode().unwrap()),
				CancellationToken::new(),
			)
			.await
			.unwrap();
		assert!(reply.bytes().is_empty());
		let report = Report {
			reset: true,
			events: vec![state("count", 1)],
		};
		plugin
			.invoke(
				REPORT,
				Bytes::from(report.encode().unwrap().remove(0)),
				CancellationToken::new(),
			)
			.await
			.unwrap();
		assert_eq!(*recorder.hellos.lock().unwrap(), [hello.capabilities]);
		assert_eq!(*recorder.reports.lock().unwrap(), [report]);

		let refused = plugin
			.invoke(REPORT, Bytes::from_static(b"[]"), CancellationToken::new())
			.await;
		assert_eq!(refused.unwrap_err(), Error::InvalidArgument);
		assert_eq!(plugin.reply_limit(COMMANDS), Ok(MAX_COMMANDS_BYTES));
		assert_eq!(plugin.reply_limit(MethodId(9)), Err(Error::Unavailable));
	}

	#[tokio::test]
	async fn a_commands_call_waits_for_the_hub() {
		let inbox = Inbox::new();
		let plugin = Arc::new(DevtoolsPlugin::new(Recorder::default(), inbox.clone()));
		let waiting = {
			let plugin = Arc::clone(&plugin);
			tokio::spawn(async move {
				plugin
					.invoke(COMMANDS, Bytes::new(), CancellationToken::new())
					.await
					.map(|reply| reply.bytes().clone())
			})
		};
		tokio::time::sleep(Duration::from_millis(20)).await;
		assert!(!waiting.is_finished());

		// Host commands never reach the guest; its own do, in order.
		inbox.push(Command::StartProfiling);
		inbox.push(set_state(1));
		inbox.push(set_state(2));
		let reply = waiting.await.unwrap().unwrap();
		assert_eq!(
			decode_commands(&reply).unwrap(),
			[set_state(1), set_state(2)]
		);
	}

	#[tokio::test]
	async fn a_wait_answers_after_its_time_on_the_hosts_clock() {
		let plugin = DevtoolsPlugin::new(Recorder::default(), Inbox::new());
		let started = tokio::time::Instant::now();
		let reply = plugin
			.invoke(
				WAIT,
				Bytes::copy_from_slice(&encode_wait(30)),
				CancellationToken::new(),
			)
			.await
			.unwrap();
		assert!(reply.bytes().is_empty());
		assert!(started.elapsed() >= Duration::from_millis(30));

		let cancellation = CancellationToken::new();
		cancellation.cancel();
		let cancelled = plugin
			.invoke(
				WAIT,
				Bytes::copy_from_slice(&encode_wait(MAX_WAIT_MS)),
				cancellation,
			)
			.await;
		assert_eq!(cancelled.unwrap_err(), Error::Cancelled);
		assert_eq!(plugin.reply_limit(WAIT), Ok(0));
	}

	#[tokio::test]
	async fn the_inbox_is_bounded() {
		let inbox = Inbox::new();
		let plugin = DevtoolsPlugin::new(Recorder::default(), inbox.clone());
		for count in 0..MAX_COMMANDS + 5 {
			inbox.push(set_state(count));
		}
		let reply = plugin
			.invoke(COMMANDS, Bytes::new(), CancellationToken::new())
			.await
			.unwrap();
		let commands = decode_commands(reply.bytes()).unwrap();
		assert_eq!(commands.len(), MAX_COMMANDS);
		assert_eq!(commands[0], set_state(5), "the oldest were dropped");
	}

	#[tokio::test]
	async fn cancelling_or_closing_ends_a_waiting_call() {
		let inbox = Inbox::new();
		let plugin = Arc::new(DevtoolsPlugin::new(Recorder::default(), inbox.clone()));
		let cancellation = CancellationToken::new();
		let cancelled = {
			let plugin = Arc::clone(&plugin);
			let cancellation = cancellation.clone();
			tokio::spawn(async move {
				plugin
					.invoke(COMMANDS, Bytes::new(), cancellation)
					.await
					.map(|_| ())
			})
		};
		tokio::time::sleep(Duration::from_millis(10)).await;
		cancellation.cancel();
		assert_eq!(cancelled.await.unwrap(), Err(Error::Cancelled));

		let closed = {
			let plugin = Arc::clone(&plugin);
			tokio::spawn(async move {
				plugin
					.invoke(COMMANDS, Bytes::new(), CancellationToken::new())
					.await
					.map(|_| ())
			})
		};
		tokio::time::sleep(Duration::from_millis(10)).await;
		plugin.shutdown().await;
		assert_eq!(closed.await.unwrap(), Err(Error::Cancelled));
		// A closed inbox takes nothing more.
		inbox.push(set_state(1));
		let after = plugin
			.invoke(COMMANDS, Bytes::new(), CancellationToken::new())
			.await;
		assert_eq!(after.unwrap_err(), Error::Cancelled);
	}
}
