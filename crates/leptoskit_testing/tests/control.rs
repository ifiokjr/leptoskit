//! Web control runs against a fake W3C driver: the protocol boundary of
//! `leptoskit_testing::control`, not a real browser.

#![cfg(all(feature = "native-ui", not(target_arch = "wasm32")))]

use std::collections::BTreeMap;
use std::io::BufRead;
use std::io::BufReader;
use std::io::ErrorKind;
use std::io::Read;
use std::io::Write;
use std::net::TcpListener;
use std::time::Duration;
use std::time::Instant;

use base64::Engine;
use leptoskit_testing::control::Browser;
use leptoskit_testing::control::Document;
use leptoskit_testing::control::Locator;
use leptoskit_testing::control::Point;
use leptoskit_testing::control::Step;
use leptoskit_testing::control::Target;
use leptoskit_testing::control::TargetKind;
use serde_json::Value;
use serde_json::json;

const ELEMENT_KEY: &str = "element-6066-11e4-a52e-4f735466cecf";

struct Reply {
	path: &'static str,
	value: Value,
}

fn reply(path: &'static str, value: Value) -> Reply {
	Reply { path, value }
}

/// One request per reply; panics on anything unexpected.
fn server(replies: Vec<Reply>) -> (String, std::thread::JoinHandle<Vec<Value>>) {
	let listener = TcpListener::bind("127.0.0.1:0").unwrap();
	listener.set_nonblocking(true).unwrap();
	let endpoint = format!("http://{}", listener.local_addr().unwrap());
	let handle = std::thread::spawn(move || {
		let mut bodies = Vec::new();
		for expected in replies {
			let deadline = Instant::now() + Duration::from_secs(10);
			let mut stream = loop {
				match listener.accept() {
					Ok((stream, _)) => break stream,
					Err(error)
						if error.kind() == ErrorKind::WouldBlock && Instant::now() < deadline =>
					{
						std::thread::sleep(Duration::from_millis(5));
					}
					Err(error) => panic!("expected {}: {error}", expected.path),
				}
			};
			stream.set_nonblocking(false).unwrap();
			stream
				.set_read_timeout(Some(Duration::from_secs(5)))
				.unwrap();
			let mut reader = BufReader::new(&stream);
			let mut first = String::new();
			reader.read_line(&mut first).unwrap();
			assert!(
				first.contains(expected.path),
				"{first} should contain {}",
				expected.path
			);
			let mut length = 0;
			loop {
				let mut line = String::new();
				reader.read_line(&mut line).unwrap();
				if line == "\r\n" {
					break;
				}
				if let Some(value) = line.to_lowercase().strip_prefix("content-length:") {
					length = value.trim().parse().unwrap();
				}
			}
			let mut bytes = vec![0; length];
			reader.read_exact(&mut bytes).unwrap();
			bodies.push(if bytes.is_empty() {
				Value::Null
			} else {
				serde_json::from_slice(&bytes).unwrap()
			});
			let body = json!({"value": expected.value}).to_string();
			write!(
				stream,
				"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: \
				 {}\r\nConnection: close\r\n\r\n{body}",
				body.len()
			)
			.unwrap();
		}
		bodies
	});
	(endpoint, handle)
}

fn png() -> Vec<u8> {
	let mut bytes = Vec::new();
	{
		let mut encoder = png::Encoder::new(&mut bytes, 2, 1);
		encoder.set_color(png::ColorType::Rgb);
		encoder
			.write_header()
			.unwrap()
			.write_image_data(&[255, 0, 0, 0, 255, 0])
			.unwrap();
	}
	bytes
}

/// The test target for a device kind, with the web fields cleared.
fn device_target(kind: TargetKind, device: &str) -> Target {
	Target {
		kind,
		device: Some(device.to_owned()),
		application: None,
		url: None,
		browser: None,
		endpoint: None,
		capabilities: BTreeMap::default(),
		headless: true,
		startup_seconds: 300,
		command_seconds: 30,
		wait_seconds: 1,
		artifacts: std::env::temp_dir().join("leptoskit-control-tests"),
	}
}

fn web_target(endpoint: &str) -> Target {
	Target {
		kind: TargetKind::Web,
		device: None,
		application: None,
		url: Some("https://example.com/journey".to_owned()),
		browser: Some(Browser::Chrome),
		endpoint: Some(endpoint.to_owned()),
		capabilities: BTreeMap::default(),
		headless: true,
		startup_seconds: 300,
		command_seconds: 30,
		wait_seconds: 1,
		artifacts: std::env::temp_dir().join("leptoskit-control-tests"),
	}
}

fn connect_replies() -> Vec<Reply> {
	vec![
		reply("GET /status ", json!({"ready": true})),
		reply("POST /session ", json!({"sessionId": "s"})),
		reply("POST /session/s/timeouts ", Value::Null),
		reply("POST /session/s/url ", Value::Null),
	]
}

#[test]
fn a_web_journey_navigates_taps_and_keeps_its_screenshot() {
	let bytes = png();
	// expect_visible, enter_text, tap, and expect_text all resolve the same
	// element; each step polls it afresh.
	let mut replies = connect_replies();
	replies.extend([
		reply("POST /session/s/elements ", json!([{ELEMENT_KEY: "one"}])),
		reply("GET /session/s/element/one/displayed ", json!(true)),
		reply("POST /session/s/elements ", json!([{ELEMENT_KEY: "one"}])),
		reply("GET /session/s/element/one/displayed ", json!(true)),
		reply("GET /session/s/element/one/enabled ", json!(true)),
		reply("POST /session/s/element/one/clear ", Value::Null),
		reply("POST /session/s/element/one/value ", Value::Null),
		reply("POST /session/s/elements ", json!([{ELEMENT_KEY: "one"}])),
		reply("GET /session/s/element/one/displayed ", json!(true)),
		reply("GET /session/s/element/one/enabled ", json!(true)),
		reply("POST /session/s/element/one/click ", Value::Null),
		reply("POST /session/s/elements ", json!([{ELEMENT_KEY: "one"}])),
		reply("GET /session/s/element/one/displayed ", json!(true)),
		reply("GET /session/s/element/one/text ", json!("Hello Ifiok")),
		reply(
			"GET /session/s/screenshot ",
			json!(base64::engine::general_purpose::STANDARD.encode(&bytes)),
		),
		reply("DELETE /session/s ", Value::Null),
	]);
	let (endpoint, server) = server(replies);
	let document = Document {
		schema_version: 1,
		target: web_target(&endpoint),
		steps: vec![
			Step::ExpectVisible {
				locator: Locator::Text("Leptoskit control".to_owned()),
			},
			Step::EnterText {
				locator: Locator::Css("#name".to_owned()),
				text: "Ifiok".to_owned(),
			},
			Step::Tap {
				locator: Locator::Css("#open".to_owned()),
			},
			Step::ExpectText {
				locator: Locator::Id("title".to_owned()),
				text: "Hello Ifiok".to_owned(),
			},
			Step::Screenshot {
				label: "after open".to_owned(),
			},
		],
	};
	let outcome = leptoskit_testing::control::run(&document);
	assert!(outcome.passed, "{:?}", outcome.error);
	assert_eq!(outcome.steps.len(), 5);
	assert_eq!(outcome.steps[2].action, "tap");
	let artifact = outcome.steps[4].artifact.as_ref().unwrap();
	assert_eq!(std::fs::read(artifact).unwrap(), bytes);
	assert!(
		artifact
			.file_name()
			.unwrap()
			.to_string_lossy()
			.contains("after-open")
	);
	let requests = server.join().unwrap();
	let capabilities = &requests[1]["capabilities"]["alwaysMatch"];
	assert_eq!(capabilities["browserName"], "chrome");
	assert_eq!(
		capabilities["goog:chromeOptions"]["args"],
		json!(["--headless=new"])
	);
	assert_eq!(requests[3]["url"], "https://example.com/journey");
	// The css locator reached the driver as a css selector, not an xpath:
	// the tap's element search, after the earlier steps' polls.
	assert_eq!(requests[11]["using"], json!("css selector"));
	assert_eq!(requests[11]["value"], json!("#open"));
	std::fs::remove_dir_all(outcome.artifacts.unwrap()).unwrap();
}

#[test]
fn a_failed_step_stops_the_journey_and_reports_it() {
	let mut replies = connect_replies();
	replies.extend([
		// Two matches make the tap fail immediately instead of polling to the
		// wait deadline, so the fake server's replies stay enumerable.
		reply(
			"POST /session/s/elements ",
			json!([{ELEMENT_KEY: "one"}, {ELEMENT_KEY: "two"}]),
		),
		reply("GET /session/s/element/one/displayed ", json!(true)),
		reply("GET /session/s/element/two/displayed ", json!(true)),
		reply("DELETE /session/s ", Value::Null),
	]);
	let (endpoint, server) = server(replies);
	let document = Document {
		schema_version: 1,
		target: web_target(&endpoint),
		steps: vec![
			Step::Tap {
				locator: Locator::Text("Ambiguous".to_owned()),
			},
			Step::Screenshot {
				label: "never".to_owned(),
			},
		],
	};
	let outcome = leptoskit_testing::control::run(&document);
	assert!(!outcome.passed);
	assert_eq!(outcome.steps.len(), 1);
	assert_eq!(outcome.steps[0].status, "error");
	assert!(
		outcome.steps[0]
			.error
			.as_deref()
			.unwrap()
			.contains("matched 2")
	);
	let artifacts = outcome.artifacts.unwrap();
	std::fs::remove_dir_all(artifacts).unwrap();
	server.join().unwrap();
}

#[test]
fn invalid_targets_report_before_any_connection() {
	let document = Document {
		schema_version: 1,
		target: Target {
			kind: TargetKind::Web,
			device: None,
			application: None,
			url: None,
			browser: None,
			endpoint: None,
			capabilities: BTreeMap::default(),
			headless: true,
			startup_seconds: 300,
			command_seconds: 30,
			wait_seconds: 15,
			artifacts: std::env::temp_dir().join("leptoskit-control-tests"),
		},
		steps: vec![],
	};
	let outcome = leptoskit_testing::control::run(&document);
	assert!(!outcome.passed);
	assert!(outcome.error.as_deref().unwrap().contains("require a url"));
	assert!(outcome.steps.is_empty());
}

/// Every driver step a document can carry, in one journey: each arm of the
/// executor's dispatch has to run at least once against a real W3C session.
#[test]
fn every_driver_step_runs_through_the_session() {
	let (endpoint, server) = server(vec![
		reply("GET /status ", json!({"ready": true})),
		reply("POST /session ", json!({"sessionId": "s"})),
		reply("POST /session/s/timeouts ", Value::Null),
		reply("POST /session/s/url ", Value::Null),
		// type_text: one key action.
		reply("POST /session/s/actions ", Value::Null),
		// press_key: one key action.
		reply("POST /session/s/actions ", Value::Null),
		// swipe: one pointer action.
		reply("POST /session/s/actions ", Value::Null),
		// perform_actions: raw sources.
		reply("POST /session/s/actions ", Value::Null),
		// expect_absent: one elements poll, no matches.
		reply("POST /session/s/elements ", json!([])),
		// alert_text.
		reply("GET /session/s/alert/text ", json!("Continue?")),
		// accept_alert reads then accepts.
		reply("GET /session/s/alert/text ", json!("Continue?")),
		reply("POST /session/s/alert/accept ", Value::Null),
		// dismiss_alert reads then dismisses.
		reply("GET /session/s/alert/text ", json!("Continue?")),
		reply("POST /session/s/alert/dismiss ", Value::Null),
		// native_command: a driver extension.
		reply("POST /session/s/execute/sync ", json!({"answer": 42})),
		// source: the page's markup.
		reply("GET /session/s/source ", json!("<html></html>")),
		reply("DELETE /session/s ", Value::Null),
	]);
	let document = Document {
		schema_version: 1,
		target: web_target(&endpoint),
		steps: vec![
			Step::TypeText {
				text: "hello".to_owned(),
			},
			Step::PressKey {
				key: "a".to_owned(),
			},
			Step::Swipe {
				start: Point { x: 10, y: 20 },
				end: Point { x: 10, y: 200 },
				duration_ms: Some(50),
			},
			Step::PerformActions {
				sources: json!([{"type": "key", "id": "raw", "actions": []}]),
			},
			Step::ExpectAbsent {
				locator: Locator::Text("Gone".to_owned()),
			},
			Step::AlertText,
			Step::AcceptAlert,
			Step::DismissAlert,
			Step::NativeCommand {
				script: "mobile: shake".to_owned(),
				arguments: json!(null),
			},
			Step::Source,
		],
	};
	let outcome = leptoskit_testing::control::run(&document);
	assert!(outcome.passed, "{:?}", outcome.error);
	assert_eq!(outcome.steps.len(), 10);
	let alert = outcome.steps[5].value.as_ref().unwrap();
	assert_eq!(alert, "Continue?");
	let command = outcome.steps[8].value.as_ref().unwrap();
	assert_eq!(command["answer"], 42);
	assert_eq!(
		outcome.steps[9].value.as_ref().and_then(Value::as_str),
		Some("<html></html>")
	);
	let requests = server.join().unwrap();
	// The swipe reached the driver as one pointer action sequence.
	let swipe = &requests[6];
	assert_eq!(swipe["actions"][0]["type"], "pointer");
	std::fs::remove_dir_all(outcome.artifacts.unwrap()).unwrap();
}

/// Every device step runs through the executor and fails fast when the
/// platform tool rejects its target — a bogus identifier on every platform,
/// with or without the tool installed.
#[test]
fn device_steps_report_tool_failures_per_step() {
	fn run_one(kind: TargetKind, device: &str, step: Step) -> leptoskit_testing::control::Outcome {
		let document = Document {
			schema_version: 1,
			target: device_target(kind, device),
			steps: vec![step],
		};
		let outcome = leptoskit_testing::control::run(&document);
		assert!(
			!outcome.passed,
			"{kind:?} step should fail against a bogus device"
		);
		assert!(!outcome.error.as_deref().unwrap_or_default().is_empty());
		outcome
	}
	let simulator = TargetKind::IosSimulator;
	let android = TargetKind::Android;
	run_one(
		simulator,
		"00000000-0000-0000-0000-000000000000",
		Step::Boot,
	);
	run_one(
		simulator,
		"00000000-0000-0000-0000-000000000000",
		Step::Shutdown,
	);
	run_one(
		simulator,
		"00000000-0000-0000-0000-000000000000",
		Step::StartApp {
			identifier: "no.such.app".to_owned(),
			arguments: Vec::new(),
			environment: BTreeMap::default(),
		},
	);
	run_one(
		simulator,
		"00000000-0000-0000-0000-000000000000",
		Step::StopApp {
			identifier: "no.such.app".to_owned(),
		},
	);
	run_one(
		simulator,
		"00000000-0000-0000-0000-000000000000",
		Step::DeviceScreenshot {
			label: "nothing".to_owned(),
		},
	);
	run_one(
		simulator,
		"00000000-0000-0000-0000-000000000000",
		Step::Logs {
			tail: Some(5),
			minutes: Some(1),
		},
	);
	run_one(
		android,
		"bogus-serial",
		Step::StartApp {
			identifier: "no.such.app".to_owned(),
			arguments: Vec::new(),
			environment: BTreeMap::default(),
		},
	);
	run_one(
		android,
		"bogus-serial",
		Step::StopApp {
			identifier: "no.such.app".to_owned(),
		},
	);
	run_one(
		android,
		"bogus-serial",
		Step::DeviceScreenshot {
			label: "nothing".to_owned(),
		},
	);
	run_one(android, "bogus-serial", Step::DeviceTap { x: 1, y: 1 });
	run_one(
		android,
		"bogus-serial",
		Step::DeviceSwipe {
			start: Point { x: 1, y: 1 },
			end: Point { x: 2, y: 2 },
			duration_ms: None,
		},
	);
	run_one(
		android,
		"bogus-serial",
		Step::DeviceKey {
			key: "BACK".to_owned(),
		},
	);
	run_one(
		android,
		"bogus-serial",
		Step::Logs {
			tail: Some(5),
			minutes: None,
		},
	);
}

/// A native (Appium) journey over the same document runner, against the fake
/// W3C driver: the control executor must reach the platform drivers it
/// already uses for native tests, not just browsers.
#[test]
fn a_native_journey_runs_through_the_shared_session() {
	let bytes = png();
	let (endpoint, server) = server(vec![
		reply("GET /status ", json!({"ready": true})),
		reply("POST /session ", json!({"sessionId": "n"})),
		reply("POST /session/n/timeouts ", Value::Null),
		// expect_visible.
		reply("POST /session/n/elements ", json!([{ELEMENT_KEY: "field"}])),
		reply("GET /session/n/element/field/displayed ", json!(true)),
		// enter_text.
		reply("POST /session/n/elements ", json!([{ELEMENT_KEY: "field"}])),
		reply("GET /session/n/element/field/displayed ", json!(true)),
		reply("GET /session/n/element/field/enabled ", json!(true)),
		reply("POST /session/n/element/field/clear ", Value::Null),
		reply("POST /session/n/element/field/value ", Value::Null),
		// type_text and press_key: one key action each.
		reply("POST /session/n/actions ", Value::Null),
		reply("POST /session/n/actions ", Value::Null),
		// swipe: one pointer action.
		reply("POST /session/n/actions ", Value::Null),
		// tap: the button, then expect_text reads the title.
		reply("POST /session/n/elements ", json!([{ELEMENT_KEY: "field"}])),
		reply("GET /session/n/element/field/displayed ", json!(true)),
		reply("GET /session/n/element/field/enabled ", json!(true)),
		reply("POST /session/n/element/field/click ", Value::Null),
		reply("POST /session/n/elements ", json!([{ELEMENT_KEY: "field"}])),
		reply("GET /session/n/element/field/displayed ", json!(true)),
		reply("GET /session/n/element/field/text ", json!("hello")),
		// expect_absent: no matches.
		reply("POST /session/n/elements ", json!([])),
		// alert_text, then accept and dismiss read before acting.
		reply("GET /session/n/alert/text ", json!("Allow?")),
		reply("GET /session/n/alert/text ", json!("Allow?")),
		reply("POST /session/n/alert/accept ", Value::Null),
		reply("GET /session/n/alert/text ", json!("Allow?")),
		reply("POST /session/n/alert/dismiss ", Value::Null),
		// native_command.
		reply("POST /session/n/execute/sync ", json!({"pressed": true})),
		reply(
			"GET /session/n/screenshot ",
			json!(base64::engine::general_purpose::STANDARD.encode(&bytes)),
		),
		reply("GET /session/n/source ", json!("<app><button/></app>")),
		reply("DELETE /session/n ", Value::Null),
	]);
	let document = Document {
		schema_version: 1,
		target: Target {
			kind: TargetKind::Macos,
			device: None,
			application: Some(leptoskit_testing::native::Application::Identifier(
				"dev.example.app".to_owned(),
			)),
			url: None,
			browser: None,
			endpoint: Some(endpoint),
			capabilities: BTreeMap::default(),
			headless: true,
			startup_seconds: 300,
			command_seconds: 30,
			wait_seconds: 1,
			artifacts: std::env::temp_dir().join("leptoskit-control-tests"),
		},
		steps: vec![
			Step::ExpectVisible {
				locator: Locator::AccessibilityId("field".to_owned()),
			},
			Step::EnterText {
				locator: Locator::AccessibilityId("field".to_owned()),
				text: "hello".to_owned(),
			},
			Step::TypeText {
				text: "more".to_owned(),
			},
			Step::PressKey {
				key: "a".to_owned(),
			},
			Step::Swipe {
				start: Point { x: 1, y: 2 },
				end: Point { x: 1, y: 9 },
				duration_ms: None,
			},
			Step::Tap {
				locator: Locator::AccessibilityId("field".to_owned()),
			},
			Step::ExpectText {
				locator: Locator::AccessibilityId("field".to_owned()),
				text: "hello".to_owned(),
			},
			Step::ExpectAbsent {
				locator: Locator::Text("Gone".to_owned()),
			},
			Step::AlertText,
			Step::AcceptAlert,
			Step::DismissAlert,
			Step::NativeCommand {
				script: "mobile: pressButton".to_owned(),
				arguments: json!("Home"),
			},
			Step::Screenshot {
				label: "typed".to_owned(),
			},
			Step::Source,
		],
	};
	let outcome = leptoskit_testing::control::run(&document);
	assert!(outcome.passed, "{:?}", outcome.error);
	assert_eq!(outcome.steps.len(), 14);
	let requests = server.join().unwrap();
	let capabilities = &requests[1]["capabilities"]["alwaysMatch"];
	assert_eq!(capabilities["appium:automationName"], "Mac2");
	assert_eq!(capabilities["appium:bundleId"], "dev.example.app");
	std::fs::remove_dir_all(outcome.artifacts.unwrap()).unwrap();
}
