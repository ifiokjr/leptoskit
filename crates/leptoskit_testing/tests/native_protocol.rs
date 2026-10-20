#![cfg(all(feature = "native-ui", not(target_arch = "wasm32")))]

use std::io::BufRead;
use std::io::BufReader;
use std::io::Read;
use std::io::Write;
use std::net::TcpListener;
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
}
fn reply(path: &'static str, value: Value) -> Reply {
	Reply { path, value }
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
						if error.kind() == std::io::ErrorKind::WouldBlock
							&& Instant::now() < deadline =>
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
