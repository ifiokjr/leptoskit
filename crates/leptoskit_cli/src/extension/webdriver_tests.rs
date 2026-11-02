use super::*;

/// Runs without an app build, so startup/install/reinstall can be checked
/// independently from the Rust-to-wasm toolchain.
#[test]
#[ignore = "needs Firefox and geckodriver; set LEPTOSKIT_FIREFOX and LEPTOSKIT_GECKODRIVER"]
fn firefox_installs_reloads_and_stops() {
	let extension = tempfile::tempdir().unwrap();
	std::fs::write(
		extension.path().join("manifest.json"),
		json!({
			"manifest_version": 3, "name": "leptoskit smoke", "version": "1.0",
			"browser_specific_settings": {"gecko": {"id": "smoke@leptoskit.test"}},
			"background": {"scripts": ["background.js"]}
		})
		.to_string(),
	)
	.unwrap();
	std::fs::write(
		extension.path().join("background.js"),
		"globalThis.__leptoskit_background_ready = true;",
	)
	.unwrap();
	std::fs::write(
		extension.path().join("probe.html"),
		"<!doctype html><title>Installed</title><p>Extension installed</p>",
	)
	.unwrap();
	let profile = tempfile::tempdir().unwrap();
	let mut driver = Driver::launch(
		ExtensionBrowser::Firefox,
		extension.path(),
		profile.path(),
		true,
	)
	.unwrap();
	driver
		.open(&format!("{}/probe.html", driver.extension_base))
		.unwrap();
	driver
		.wait("document.title === 'Installed'", Duration::from_secs(5))
		.unwrap();
	std::fs::write(
		extension.path().join("probe.html"),
		"<!doctype html><title>Reloaded</title>",
	)
	.unwrap();
	driver.reinstall(extension.path()).unwrap();
	driver
		.open(&format!("{}/probe.html", driver.extension_base))
		.unwrap();
	driver
		.wait("document.title === 'Reloaded'", Duration::from_secs(5))
		.unwrap();
}

/// Fake geckodriver: validates negotiation and lifetime without requiring a
/// graphical environment or a particular Firefox installation.
#[test]
fn firefox_session_installs_retries_and_tears_down() {
	use std::net::TcpListener;
	let http = TcpListener::bind("127.0.0.1:0").unwrap();
	let ws = TcpListener::bind("127.0.0.1:0").unwrap();
	let base = format!("http://{}", http.local_addr().unwrap());
	let socket = format!("ws://{}/session", ws.local_addr().unwrap());
	let extension = tempfile::tempdir().unwrap();
	let path = extension.path().to_path_buf();
	let web_socket = std::thread::spawn(move || fake_extension_socket(&ws, &path));
	let requests = std::thread::spawn(move || fake_webdriver(&http, &socket));
	let mut driver = Driver {
		process: None,
		isolated: false,
		base,
		session: String::new(),
		extension_base: String::new(),
		bidi: None,
		addon: None,
		agent: ureq::Agent::config_builder()
			.timeout_global(Some(Duration::from_secs(3)))
			.build()
			.into(),
	};
	driver
		.start_firefox(
			extension.path(),
			Path::new("/fake/firefox"),
			"test@example.com",
			"test-uuid",
			true,
		)
		.unwrap();
	assert_eq!(driver.extension_base, "moz-extension://test-uuid");
	driver.open("moz-extension://test-uuid/probe.html").unwrap();
	assert_eq!(driver.script("return true").unwrap(), true);
	let error = driver.reinstall(extension.path()).unwrap_err().to_string();
	assert!(error.contains("invalid manifest"), "{error}");
	// After uninstall succeeds and installation fails, the next rebuild can
	// retry installation rather than trying to uninstall a missing add-on.
	driver.reinstall(extension.path()).unwrap();
	drop(driver);
	requests.join().unwrap();
	web_socket.join().unwrap();
}

fn fake_extension_socket(listener: &std::net::TcpListener, path: &Path) {
	let (stream, _) = listener.accept().unwrap();
	stream
		.set_read_timeout(Some(Duration::from_secs(5)))
		.unwrap();
	let mut socket = tungstenite::accept(stream).unwrap();

	for (index, method) in [
		"webExtension.install",
		"webExtension.uninstall",
		"webExtension.install",
		"webExtension.install",
	]
	.iter()
	.enumerate()
	{
		let message = socket.read().unwrap();
		let request: Value = serde_json::from_str(message.to_text().unwrap()).unwrap();
		assert_eq!(request["method"], *method);
		let params = if *method == "webExtension.uninstall" {
			json!({"extension": "test@example.com"})
		} else {
			json!({"extensionData": {"type": "path", "path": path}})
		};
		assert_eq!(request["params"], params);
		let response = if index == 2 {
			json!({"id":request["id"], "type":"error", "error":"invalid web extension", "message":"invalid manifest"})
		} else {
			json!({"id":request["id"], "type":"success", "result":{"extension":"test@example.com"}})
		};
		socket
			.send(tungstenite::Message::Text(response.to_string().into()))
			.unwrap();
	}
}

fn fake_webdriver(listener: &std::net::TcpListener, socket: &str) {
	use std::io::Write as _;

	for expected in [
		"POST /session HTTP/1.1",
		"POST /session/test/url HTTP/1.1",
		"POST /session/test/execute/async HTTP/1.1",
		"DELETE /session/test HTTP/1.1",
	] {
		let (mut stream, _) = listener.accept().unwrap();
		stream
			.set_read_timeout(Some(Duration::from_secs(5)))
			.unwrap();
		let (line, request) = read_request(&mut stream);
		assert_eq!(line.trim(), expected);
		let result = match expected {
			"POST /session HTTP/1.1" => {
				let caps = &request["capabilities"]["alwaysMatch"];
				assert_eq!(caps["browserName"], "firefox");
				assert_eq!(caps["webSocketUrl"], true);
				assert_eq!(caps["moz:firefoxOptions"]["args"], json!(["-headless"]));
				assert_eq!(caps["moz:firefoxOptions"]["binary"], "/fake/firefox");
				let uuids: Value = serde_json::from_str(
					caps["moz:firefoxOptions"]["prefs"]["extensions.webextensions.uuids"]
						.as_str()
						.unwrap(),
				)
				.unwrap();
				assert_eq!(uuids, json!({"test@example.com":"test-uuid"}));
				assert_eq!(
					caps["moz:firefoxOptions"]["prefs"]
						["extensions.webextensions.keepUuidOnUninstall"],
					true
				);
				assert_eq!(
					caps["moz:firefoxOptions"]["prefs"]
						["extensions.webextensions.keepStorageOnUninstall"],
					true
				);
				json!({"sessionId":"test", "capabilities":{"webSocketUrl":socket}})
			}

			"POST /session/test/url HTTP/1.1" => {
				assert_eq!(request["url"], "moz-extension://test-uuid/probe.html");
				Value::Null
			}

			"POST /session/test/execute/async HTTP/1.1" => {
				assert_eq!(request["script"], "return true");
				json!(true)
			}

			_ => Value::Null,
		};
		let body = json!({"value": result}).to_string();
		write!(
			stream,
			"HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: \
			 {}\r\nconnection: close\r\n\r\n{body}",
			body.len()
		)
		.unwrap();
	}
}

// Read exactly one HTTP request; the fake server closes each connection
// after replying so its request order and session teardown are observable.
fn read_request(stream: &mut std::net::TcpStream) -> (String, Value) {
	use std::io::BufRead as _;
	use std::io::Read as _;
	let mut reader = std::io::BufReader::new(stream);
	let mut first = String::new();
	reader.read_line(&mut first).unwrap();
	let mut length = 0;

	loop {
		let mut line = String::new();
		reader.read_line(&mut line).unwrap();

		if line == "\r\n" {
			break;
		}

		if let Some((name, value)) = line.split_once(':')
			&& name.eq_ignore_ascii_case("content-length")
		{
			length = value.trim().parse().unwrap();
		}
	}

	let mut body = vec![0; length];
	reader.read_exact(&mut body).unwrap();
	(
		first,
		if body.is_empty() {
			Value::Null
		} else {
			serde_json::from_slice(&body).unwrap()
		},
	)
}

/// Keeps the Chromium path checked when sharing the transport with Firefox.
#[test]
#[ignore = "needs Chrome for Testing and chromedriver"]
fn chromium_still_loads_an_unpacked_extension() {
	let extension = tempfile::tempdir().unwrap();
	std::fs::write(
		extension.path().join("manifest.json"),
		json!({
			"manifest_version": 3, "name": "leptoskit smoke", "version": "1.0",
			"background": {"service_worker": "background.js"}
		})
		.to_string(),
	)
	.unwrap();
	std::fs::write(
		extension.path().join("background.js"),
		"chrome.runtime.onMessage.addListener(() => {});",
	)
	.unwrap();
	std::fs::write(
		extension.path().join("probe.html"),
		"<!doctype html><title>Installed</title>",
	)
	.unwrap();
	let profile = tempfile::tempdir().unwrap();
	let driver = Driver::launch(
		ExtensionBrowser::Chromium,
		extension.path(),
		profile.path(),
		true,
	)
	.unwrap();
	driver
		.open(&format!("{}/probe.html", driver.extension_base))
		.unwrap();
	driver
		.wait("document.title === 'Installed'", Duration::from_secs(5))
		.unwrap();
}
