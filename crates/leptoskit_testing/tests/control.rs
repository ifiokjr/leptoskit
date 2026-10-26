//! Web control runs against a fake W3C driver: the protocol boundary of
//! `leptoskit_testing::control`, not a real browser.

#![cfg(all(feature = "native-ui", not(target_arch = "wasm32")))]

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

fn web_target(endpoint: &str) -> Target {
	Target {
		kind: TargetKind::Web,
		device: None,
		application: None,
		url: Some("https://example.com/journey".to_owned()),
		browser: Some(Browser::Chrome),
		endpoint: Some(endpoint.to_owned()),
		capabilities: Default::default(),
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
	let mut replies = connect_replies();
	replies.extend([
		reply("POST /session/s/elements ", json!([{ELEMENT_KEY: "one"}])),
		reply("GET /session/s/element/one/displayed ", json!(true)),
		reply("GET /session/s/element/one/enabled ", json!(true)),
		reply("POST /session/s/element/one/click ", Value::Null),
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
			Step::Tap {
				locator: Locator::Css("#open".to_owned()),
			},
			Step::Screenshot {
				label: "after open".to_owned(),
			},
		],
	};
	let outcome = leptoskit_testing::control::run(document);
	assert!(outcome.passed, "{:?}", outcome.error);
	assert_eq!(outcome.steps.len(), 2);
	assert_eq!(outcome.steps[0].action, "tap");
	let artifact = outcome.steps[1].artifact.as_ref().unwrap();
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
	// The css locator reached the driver as a css selector, not an xpath.
	assert_eq!(requests[4]["using"], json!("css selector"));
	assert_eq!(requests[4]["value"], json!("#open"));
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
	let outcome = leptoskit_testing::control::run(document);
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
			capabilities: Default::default(),
			headless: true,
			startup_seconds: 300,
			command_seconds: 30,
			wait_seconds: 15,
			artifacts: std::env::temp_dir().join("leptoskit-control-tests"),
		},
		steps: vec![],
	};
	let outcome = leptoskit_testing::control::run(document);
	assert!(!outcome.passed);
	assert!(outcome.error.as_deref().unwrap().contains("require a url"));
	assert!(outcome.steps.is_empty());
}
