#![cfg(all(feature = "native-ui", not(target_arch = "wasm32")))]

use std::io::BufRead;
use std::io::BufReader;
use std::io::ErrorKind;
use std::io::Read;
use std::io::Write;
use std::net::TcpListener;
use std::net::TcpStream;
use std::time::Duration;
use std::time::Instant;

use base64::Engine;
use leptoskit_testing::native::Application;
use leptoskit_testing::native::Error;
use leptoskit_testing::native::Locator;
use leptoskit_testing::native::NativeTester;
use leptoskit_testing::native::Options;
use leptoskit_testing::native::Platform;
use serde_json::Value;
use serde_json::json;

struct Reply {
	path: &'static str,
	value: Value,
	/// A raw HTTP status and plain-text body instead of a W3C JSON envelope.
	raw: Option<(&'static str, &'static str)>,
}
fn reply(path: &'static str, value: Value) -> Reply {
	Reply {
		path,
		value,
		raw: None,
	}
}
fn raw(path: &'static str, status: &'static str, body: &'static str) -> Reply {
	Reply {
		path,
		value: Value::Null,
		raw: Some((status, body)),
	}
}

// This server tests the protocol boundary, not OS automation. The separate
// native conformance jobs keep the real platform driver and native UI.
fn server(replies: Vec<Reply>) -> (Options, std::thread::JoinHandle<Vec<Value>>) {
	let listener = TcpListener::bind("127.0.0.1:0").unwrap();
	listener.set_nonblocking(true).unwrap();
	let endpoint = format!("http://{}/wd", listener.local_addr().unwrap());
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
			let (status, content_type, body) = match expected.raw {
				Some((status, body)) => (status, "text/html", body.to_owned()),
				None => {
					(
						"200 OK",
						"application/json",
						json!({"value": expected.value}).to_string(),
					)
				}
			};
			write!(
				stream,
				"HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: \
				 {}\r\nConnection: close\r\n\r\n{body}",
				body.len()
			)
			.unwrap();
		}
		bodies
	});
	let mut options = Options::new(endpoint, Platform::Windows, Application::Desktop);
	options.artifacts = std::env::temp_dir().join("leptoskit-native-protocol-tests");
	options.wait_seconds = 1;
	(options, handle)
}

fn setup() -> Vec<Reply> {
	vec![
		reply("GET /wd/status ", json!({"ready": true})),
		reply("POST /wd/session ", json!({"sessionId":"s"})),
		reply("POST /wd/session/s/timeouts ", Value::Null),
	]
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

#[test]
fn ambiguous_controls_are_not_clicked_and_failure_report_embeds_real_png() {
	let bytes = png();
	let mut replies = setup();
	replies.extend([
        reply("POST /wd/session/s/elements ", json!([{"element-6066-11e4-a52e-4f735466cecf":"one"},{"element-6066-11e4-a52e-4f735466cecf":"two"}])),
        reply("GET /wd/session/s/element/one/displayed ", json!(true)),
        reply("GET /wd/session/s/element/two/displayed ", json!(true)),
        reply("GET /wd/session/s/screenshot ", json!(base64::engine::general_purpose::STANDARD.encode(&bytes))),
        reply("GET /wd/session/s/source ", json!("<desktop><button name='Open'/></desktop>")),
        reply("DELETE /wd/session/s ", Value::Null),
    ]);
	let (options, server) = server(replies);
	let tester = NativeTester::connect(options).unwrap();
	let directory = tester.artifacts().to_path_buf();
	let result = tester.tap(&Locator::text("<Open>"));
	assert!(matches!(result, Err(Error::Ambiguous { count: 2, .. })));
	assert!(tester.finish(result).is_err());
	let requests = server.join().unwrap();
	assert_eq!(
		requests[1]["capabilities"]["alwaysMatch"]["appium:shouldCloseApp"],
		false
	);
	assert_eq!(
		std::fs::read(directory.join("capture-001.png")).unwrap(),
		bytes
	);
	let html = std::fs::read_to_string(directory.join("index.html")).unwrap();
	assert!(html.contains("<img src=\"capture-001.png\""));
	assert!(html.contains("&lt;Open&gt;"));
	assert!(!html.contains("<Open>"));
	std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn corrupt_screenshot_and_cleanup_failure_both_fail_the_run() {
	let mut replies = setup();
	replies.extend([
		reply("GET /wd/session/s/screenshot ", json!("not base64")),
		reply(
			"DELETE /wd/session/s ",
			json!({"error":"unknown error","message":"cleanup failed"}),
		),
	]);
	let (options, server) = server(replies);
	let tester = NativeTester::connect(options).unwrap();
	let directory = tester.artifacts().to_path_buf();
	let failure = tester
		.finish::<()>(Err(Error::Assertion("original failure".to_owned())))
		.unwrap_err()
		.to_string();
	assert!(failure.contains("original failure"));
	assert!(failure.contains("invalid screenshot base64"));
	assert!(failure.contains("cleanup failed"));
	assert!(!directory.join("capture-001.png").exists());
	server.join().unwrap();
	std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn mutation_errors_are_not_retried() {
	let mut replies = setup();
	replies.extend([
		reply(
			"POST /wd/session/s/elements ",
			json!([{"element-6066-11e4-a52e-4f735466cecf":"one"}]),
		),
		reply("GET /wd/session/s/element/one/displayed ", json!(true)),
		reply("GET /wd/session/s/element/one/enabled ", json!(true)),
		reply(
			"POST /wd/session/s/element/one/click ",
			json!({"error":"stale element reference","message":"gone during click"}),
		),
		reply("DELETE /wd/session/s ", Value::Null),
	]);
	let (options, server) = server(replies);
	let tester = NativeTester::connect(options).unwrap();
	let directory = tester.artifacts().to_path_buf();
	assert!(
		matches!(tester.tap(&Locator::text("Open")), Err(Error::Driver { code, .. }) if code == "stale element reference")
	);
	tester.finish(Ok(())).unwrap();
	server.join().unwrap();
	std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn linux_driver_uses_its_native_timeout_contract_and_balanced_actions() {
	let replies = vec![
		reply("GET /wd/status ", json!({"ready":"true"})),
		reply("POST /wd/session ", json!({"sessionId":"s"})),
		reply("POST /wd/session/s/timeouts/implicit_wait ", Value::Null),
		reply("POST /wd/session/s/actions ", Value::Null),
		reply("DELETE /wd/session/s ", Value::Null),
	];
	let (mut options, server) = server(replies);
	options.platform = Platform::Linux;
	let tester = NativeTester::connect(options).unwrap();
	let directory = tester.artifacts().to_path_buf();
	tester.perform_actions(&json!([{"type":"key","id":"keyboard","actions":[{"type":"keyDown","value":"a"},{"type":"keyUp","value":"a"}]}])).unwrap();
	tester.finish(Ok(())).unwrap();
	let bodies = server.join().unwrap();
	assert_eq!(bodies[2], json!({"ms":100}));
	std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn panicking_assertions_retain_evidence_close_the_session_and_resume_the_panic() {
	let bytes = png();
	let mut replies = setup();
	replies.extend([
		reply(
			"GET /wd/session/s/screenshot ",
			json!(base64::engine::general_purpose::STANDARD.encode(&bytes)),
		),
		reply("GET /wd/session/s/source ", json!("<desktop/>")),
		reply("DELETE /wd/session/s ", Value::Null),
	]);
	let (options, server) = server(replies);
	let directory = std::sync::Mutex::new(None);
	let result = std::panic::catch_unwind(|| {
		NativeTester::run::<()>(options, |tester| {
			*directory.lock().unwrap() = Some(tester.artifacts().to_path_buf());
			panic!("native assertion failed");
		})
	});
	assert_eq!(
		result.unwrap_err().downcast_ref::<&str>(),
		Some(&"native assertion failed")
	);
	let directory = directory.into_inner().unwrap().unwrap();
	let html = std::fs::read_to_string(directory.join("index.html")).unwrap();
	assert!(html.contains("scenario panicked: native assertion failed"));
	assert_eq!(
		std::fs::read(directory.join("capture-001.png")).unwrap(),
		bytes
	);
	server.join().unwrap();
	std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn startup_failures_keep_the_http_status_and_write_a_report() {
	let replies = vec![
		reply("GET /wd/status ", json!({"ready": true})),
		raw(
			"POST /wd/session ",
			"502 Bad Gateway",
			"<html>upstream driver crashed</html>",
		),
	];
	let (options, server) = server(replies);
	let parent = options.artifacts.clone();
	let before: std::collections::BTreeSet<_> = std::fs::read_dir(&parent)
		.map(|entries| entries.map(|entry| entry.unwrap().path()).collect())
		.unwrap_or_default();
	let Err(Error::Evidence { directory, source }) = NativeTester::connect(options) else {
		panic!("startup failure must carry its evidence directory");
	};
	assert!(
		matches!(*source, Error::Response(ref message) if message.contains("HTTP 502 Bad Gateway") && message.contains("upstream driver crashed"))
	);
	assert!(!before.contains(&directory));
	let html = std::fs::read_to_string(directory.join("index.html")).unwrap();
	assert!(html.contains("HTTP 502 Bad Gateway"));
	assert!(!html.contains("capture-001"));
	server.join().unwrap();
	std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn failed_tree_capture_is_not_linked_and_the_primary_error_keeps_its_variant() {
	let bytes = png();
	let mut replies = setup();
	replies.extend([
		reply(
			"GET /wd/session/s/screenshot ",
			json!(base64::engine::general_purpose::STANDARD.encode(&bytes)),
		),
		reply(
			"GET /wd/session/s/source ",
			json!({"error":"unknown error","message":"tree unavailable"}),
		),
		reply("DELETE /wd/session/s ", Value::Null),
	]);
	let (options, server) = server(replies);
	let tester = NativeTester::connect(options).unwrap();
	let directory = tester.artifacts().to_path_buf();
	let Err(Error::Evidence { source, .. }) = tester.finish::<()>(Err(Error::Ambiguous {
		locator: Locator::text("Open"),
		count: 2,
	})) else {
		panic!("a failed run must carry its evidence directory");
	};
	let Error::Combined { primary, secondary } = *source else {
		panic!("the failed tree capture must be reported with the original failure");
	};
	assert!(matches!(*primary, Error::Ambiguous { count: 2, .. }));
	assert!(secondary.to_string().contains("tree unavailable"));
	let html = std::fs::read_to_string(directory.join("index.html")).unwrap();
	assert!(!html.contains("capture-001.xml"));
	server.join().unwrap();
	std::fs::remove_dir_all(directory).unwrap();
}

fn element(id: &str) -> Value {
	json!([{ "element-6066-11e4-a52e-4f735466cecf": id }])
}

#[test]
fn serialized_suites_drive_every_step_through_the_driver_once() {
	let bytes = png();
	let mut replies = setup();
	replies.extend([
		reply("POST /wd/session/s/elements ", element("name")),
		reply("GET /wd/session/s/element/name/displayed ", json!(true)),
		reply("GET /wd/session/s/element/name/enabled ", json!(true)),
		reply("POST /wd/session/s/element/name/clear ", Value::Null),
		reply("POST /wd/session/s/element/name/value ", Value::Null),
		reply("POST /wd/session/s/elements ", element("greet")),
		reply("GET /wd/session/s/element/greet/displayed ", json!(true)),
		reply("GET /wd/session/s/element/greet/enabled ", json!(true)),
		reply("POST /wd/session/s/element/greet/click ", Value::Null),
		reply("POST /wd/session/s/elements ", element("greeting")),
		reply("GET /wd/session/s/element/greeting/displayed ", json!(true)),
		reply(
			"GET /wd/session/s/element/greeting/text ",
			json!("Hello, Ada"),
		),
		reply("POST /wd/session/s/elements ", json!([])),
		reply(
			"GET /wd/session/s/alert/text ",
			json!({"error": "no such alert", "message": "not yet"}),
		),
		reply(
			"GET /wd/session/s/alert/text ",
			json!("Allow notifications?"),
		),
		reply("POST /wd/session/s/alert/accept ", Value::Null),
		reply("GET /wd/session/s/alert/text ", json!("Share location?")),
		reply("POST /wd/session/s/alert/dismiss ", Value::Null),
		reply("POST /wd/session/s/execute/sync ", Value::Null),
		reply("POST /wd/session/s/actions ", Value::Null),
		reply("POST /wd/session/s/elements ", element("done")),
		reply("GET /wd/session/s/element/done/displayed ", json!(true)),
		reply(
			"GET /wd/session/s/screenshot ",
			json!(base64::engine::general_purpose::STANDARD.encode(&bytes)),
		),
		reply("GET /wd/session/s/source ", json!("<desktop/>")),
		reply("DELETE /wd/session/s ", Value::Null),
	]);
	let (options, server) = server(replies);
	let suite: leptoskit_testing::native::suite::Suite = serde_json::from_value(json!({
		"session": options,
		"steps": [
			{"action": "enter_text", "locator": {"by": "text", "value": "Name"}, "text": "Ada"},
			{"action": "tap", "locator": {"by": "accessibility_id", "value": "greet"}},
			{"action": "expect_text", "locator": {"by": "text", "value": "Greeting"}, "text": "Hello, Ada"},
			{"action": "expect_absent", "locator": {"by": "text", "value": "Spinner"}},
			{"action": "accept_alert"},
			{"action": "dismiss_alert"},
			{"action": "native_command", "script": "mobile: pressButton", "arguments": {"name": "home"}},
			{"action": "perform_actions", "sources": []},
			{"action": "expect_visible", "locator": {"by": "text", "value": "Done"}},
			{"action": "screenshot", "label": "Finished journey"},
		],
	}))
	.unwrap();
	let directory = suite.run().unwrap();
	let bodies = server.join().unwrap();
	assert_eq!(bodies[7], json!({"text": "Ada", "value": ["A", "d", "a"]}));
	assert_eq!(
		bodies[21],
		json!({"script": "mobile: pressButton", "args": [{"name": "home"}]})
	);
	assert_eq!(
		std::fs::read(directory.join("capture-001.png")).unwrap(),
		bytes
	);
	let html = std::fs::read_to_string(directory.join("index.html")).unwrap();
	assert!(html.contains("Finished journey"));
	assert!(!html.contains("Failed"));
	std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn empty_suites_are_rejected_before_any_driver_request() {
	let suite: leptoskit_testing::native::suite::Suite = serde_json::from_value(json!({
		"session": Options::new("http://127.0.0.1:9", Platform::Linux, Application::Desktop),
		"steps": [],
	}))
	.unwrap();
	assert!(matches!(suite.run(), Err(Error::Configuration(_))));
}

/// A driver that answers by path for as long as the session lives, for
/// waits whose number of polls depends on timing.
fn polling_server(answer: fn(&str) -> Value) -> (Options, std::thread::JoinHandle<()>) {
	let listener = TcpListener::bind("127.0.0.1:0").unwrap();
	let endpoint = format!("http://{}/wd", listener.local_addr().unwrap());
	let handle = std::thread::spawn(move || {
		loop {
			let (stream, _) = listener.accept().unwrap();
			let Some(first) = unless_abandoned(read_request(&stream)) else {
				continue;
			};
			unless_abandoned(respond(&stream, &answer(&first)));
			if first.starts_with("DELETE ") {
				return;
			}
		}
	});
	let mut options = Options::new(endpoint, Platform::Windows, Application::Desktop);
	options.artifacts = std::env::temp_dir().join("leptoskit-native-protocol-tests");
	options.wait_seconds = 1;
	(options, handle)
}

/// Reads one request and returns its request line. A connection that ends
/// before the request does fails with `UnexpectedEof`.
fn read_request(stream: &TcpStream) -> std::io::Result<String> {
	fn read_line(reader: &mut impl BufRead) -> std::io::Result<String> {
		let mut line = String::new();
		match reader.read_line(&mut line)? {
			0 => Err(ErrorKind::UnexpectedEof.into()),
			_ => Ok(line),
		}
	}
	stream.set_read_timeout(Some(Duration::from_secs(5)))?;
	let mut reader = BufReader::new(stream);
	let first = read_line(&mut reader)?;
	let mut length = 0;
	loop {
		let line = read_line(&mut reader)?;
		if line == "\r\n" {
			break;
		}
		if let Some(value) = line.to_lowercase().strip_prefix("content-length:") {
			length = value.trim().parse().unwrap();
		}
	}
	reader.read_exact(&mut vec![0; length])?;
	Ok(first)
}

fn respond(stream: &TcpStream, value: &Value) -> std::io::Result<()> {
	let body = json!({ "value": value }).to_string();
	write!(
		&*stream,
		"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: \
		 close\r\n\r\n{body}",
		body.len()
	)
}

/// Returns `None` when the client gave up on the connection. A wait abandons
/// the observation its deadline cuts short, so on a loaded host a poll may
/// close or reset at any point before its response is read. Any other I/O
/// failure is a broken test driver.
fn unless_abandoned<T>(result: std::io::Result<T>) -> Option<T> {
	match result {
		Ok(value) => Some(value),
		Err(error)
			if matches!(
				error.kind(),
				ErrorKind::UnexpectedEof
					| ErrorKind::ConnectionReset
					| ErrorKind::ConnectionAborted
					| ErrorKind::BrokenPipe
			) =>
		{
			None
		}
		Err(error) => panic!("test driver connection failed: {error}"),
	}
}

#[test]
fn disabled_controls_time_out_with_an_explanation() {
	let (options, server) = polling_server(|request| {
		let path = request.split(' ').nth(1).unwrap();
		match path {
			"/wd/status" => json!({"ready": true}),
			"/wd/session" => json!({"sessionId": "s"}),
			"/wd/session/s/elements" => element("save"),
			"/wd/session/s/element/save/displayed" => json!(true),
			"/wd/session/s/element/save/enabled" => json!(false),
			"/wd/session/s/screenshot" => {
				json!(base64::engine::general_purpose::STANDARD.encode(png()))
			}
			"/wd/session/s/source" => json!("<desktop/>"),
			_ => Value::Null,
		}
	});
	// On a loaded host the wait's deadline can abandon a poll before the
	// driver reads it. That connection must not take the driver down.
	let address = options
		.endpoint
		.trim_start_matches("http://")
		.trim_end_matches("/wd");
	drop(TcpStream::connect(address).unwrap());
	let tester = NativeTester::connect(options).unwrap();
	let directory = tester.artifacts().to_path_buf();
	let error = tester.tap(&Locator::text("Save")).unwrap_err();
	assert!(
		matches!(&error, Error::Timeout(message) if message.contains("to be visible and enabled")),
		"{error}"
	);
	assert!(tester.finish::<()>(Err(error)).is_err());
	server.join().unwrap();
	let html = std::fs::read_to_string(directory.join("index.html")).unwrap();
	assert!(html.contains("to be visible and enabled"));
	std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn responses_cut_short_by_the_wait_deadline_time_out_the_wait() {
	// A loaded driver can stall after sending part of a response. Running out
	// of wait while reading the body is the wait's timeout, not an I/O error.
	let listener = TcpListener::bind("127.0.0.1:0").unwrap();
	let endpoint = format!("http://{}/wd", listener.local_addr().unwrap());
	let server = std::thread::spawn(move || {
		loop {
			let (stream, _) = listener.accept().unwrap();
			let Some(first) = unless_abandoned(read_request(&stream)) else {
				continue;
			};
			let value = match first.split(' ').nth(1).unwrap() {
				"/wd/status" => json!({"ready": true}),
				"/wd/session" => json!({"sessionId": "s"}),
				"/wd/session/s/elements" => {
					write!(
						&stream,
						"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: \
						 64\r\nConnection: close\r\n\r\n{{\"value\":"
					)
					.unwrap();
					// Hold the body open until the client gives up on it.
					stream.set_read_timeout(None).unwrap();
					let held = (&stream).read(&mut [0]);
					assert!(
						matches!(held, Ok(0)) || unless_abandoned(held).is_none(),
						"the client must abandon the stalled response"
					);
					continue;
				}
				"/wd/session/s/screenshot" => {
					json!(base64::engine::general_purpose::STANDARD.encode(png()))
				}
				"/wd/session/s/source" => json!("<desktop/>"),
				_ => Value::Null,
			};
			unless_abandoned(respond(&stream, &value));
			if first.starts_with("DELETE ") {
				return;
			}
		}
	});
	let mut options = Options::new(endpoint, Platform::Windows, Application::Desktop);
	options.artifacts = std::env::temp_dir().join("leptoskit-native-protocol-tests");
	options.wait_seconds = 1;
	let tester = NativeTester::connect(options).unwrap();
	let directory = tester.artifacts().to_path_buf();
	let error = tester.tap(&Locator::text("Save")).unwrap_err();
	assert!(
		matches!(&error, Error::Timeout(message) if message.contains("to be visible and enabled")),
		"{error}"
	);
	assert!(tester.finish::<()>(Err(error)).is_err());
	server.join().unwrap();
	std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn driver_readiness_waits_through_unready_status_and_rejects_malformed_status() {
	let replies = vec![
		reply("GET /wd/status ", json!({"ready": false})),
		reply("GET /wd/status ", json!({"ready": "false"})),
		reply("GET /wd/status ", json!({"ready": true})),
		reply("POST /wd/session ", json!({"sessionId": "s"})),
		reply("POST /wd/session/s/timeouts ", Value::Null),
		reply("DELETE /wd/session/s ", Value::Null),
	];
	let (options, driver) = server(replies);
	let tester = NativeTester::connect(options).unwrap();
	let directory = tester.artifacts().to_path_buf();
	tester.finish(Ok(())).unwrap();
	driver.join().unwrap();
	std::fs::remove_dir_all(directory).unwrap();

	let (options, driver) = server(vec![reply("GET /wd/status ", json!({"build": {}}))]);
	let Err(Error::Evidence { directory, source }) = NativeTester::connect(options) else {
		panic!("a malformed status must fail startup with evidence");
	};
	assert!(matches!(*source, Error::Response(ref message) if message.contains("ready")));
	driver.join().unwrap();
	std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn unreachable_drivers_fail_within_the_startup_deadline() {
	let port = TcpListener::bind("127.0.0.1:0")
		.unwrap()
		.local_addr()
		.unwrap()
		.port();
	let mut options = Options::new(
		format!("http://127.0.0.1:{port}/wd"),
		Platform::Windows,
		Application::Desktop,
	);
	options.artifacts = std::env::temp_dir().join("leptoskit-native-protocol-tests");
	options.startup_seconds = 1;
	let start = Instant::now();
	let Err(Error::Evidence { directory, source }) = NativeTester::connect(options) else {
		panic!("an unreachable driver must fail startup with evidence");
	};
	assert!(matches!(*source, Error::Timeout(_)), "{source}");
	assert!(start.elapsed() < Duration::from_secs(5));
	std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn endpoints_and_responses_are_validated() {
	for endpoint in ["ftp://127.0.0.1/wd", "http://127.0.0.1/wd?x=1", "not a url"] {
		let options = Options::new(endpoint, Platform::Windows, Application::Desktop);
		assert!(
			matches!(NativeTester::connect(options), Err(Error::Configuration(_))),
			"{endpoint}"
		);
	}
	let replies = vec![
		reply("GET /wd/status ", json!({"ready": true})),
		raw("POST /wd/session ", "200 OK", "not json"),
	];
	let (options, server) = server(replies);
	let Err(Error::Evidence { directory, source }) = NativeTester::connect(options) else {
		panic!("a non-JSON success must fail startup");
	};
	assert!(matches!(*source, Error::Json(_)), "{source}");
	server.join().unwrap();
	std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn slow_observations_share_the_wait_deadline_not_the_command_deadline() {
	// An iOS accessibility snapshot can stall for longer than one command's
	// deadline while the driver is still answering. The lookup must succeed
	// within the wait instead of failing on the per-command timeout.
	let (mut options, server) = polling_server(|request| {
		let path = request.split(' ').nth(1).unwrap();
		match path {
			"/wd/status" => json!({"ready": true}),
			"/wd/session" => json!({"sessionId": "s"}),
			"/wd/session/s/elements" => {
				std::thread::sleep(Duration::from_millis(1_500));
				element("greet")
			}
			"/wd/session/s/element/greet/displayed" | "/wd/session/s/element/greet/enabled" => {
				json!(true)
			}
			_ => Value::Null,
		}
	});
	options.command_seconds = 1;
	options.wait_seconds = 5;
	let tester = NativeTester::connect(options).unwrap();
	let directory = tester.artifacts().to_path_buf();
	let start = Instant::now();
	tester.tap(&Locator::accessibility_id("greet")).unwrap();
	assert!(start.elapsed() >= Duration::from_millis(1_500));
	tester.finish(Ok(())).unwrap();
	server.join().unwrap();
	std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn slow_actions_are_not_abandoned_while_the_driver_may_still_run_them() {
	// On a loaded iOS simulator, typing or tapping can outlast one command's
	// deadline while WebDriverAgent is still performing it. Actions are sent
	// once, so the client must wait for the driver's answer within its bound.
	let (mut options, server) = polling_server(|request| {
		let path = request.split(' ').nth(1).unwrap();
		match path {
			"/wd/status" => json!({"ready": true}),
			"/wd/session" => json!({"sessionId": "s"}),
			"/wd/session/s/elements" => element("greet"),
			"/wd/session/s/element/greet/displayed" | "/wd/session/s/element/greet/enabled" => {
				json!(true)
			}
			"/wd/session/s/element/greet/click" => {
				std::thread::sleep(Duration::from_millis(1_500));
				Value::Null
			}
			_ => Value::Null,
		}
	});
	options.command_seconds = 1;
	options.wait_seconds = 3;
	let tester = NativeTester::connect(options).unwrap();
	let directory = tester.artifacts().to_path_buf();
	let start = Instant::now();
	tester.tap(&Locator::accessibility_id("greet")).unwrap();
	assert!(start.elapsed() >= Duration::from_millis(1_500));
	tester.finish(Ok(())).unwrap();
	server.join().unwrap();
	std::fs::remove_dir_all(directory).unwrap();
}
