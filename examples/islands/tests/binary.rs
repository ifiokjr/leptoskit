//! The server binary as `leptoskit serve` runs it: configured through the
//! `LEPTOS_*` environment, serving pages, and stopping cleanly on Ctrl-C or
//! on the SIGTERM a container runtime sends.

#![cfg(all(unix, feature = "ssr"))]

use std::io::BufRead;
use std::io::BufReader;
use std::io::Read;
use std::io::Write;
use std::net::TcpListener;
use std::net::TcpStream;
use std::process::Command;
use std::process::Stdio;
use std::time::Duration;
use std::time::Instant;

#[test]
fn the_server_serves_pages_and_stops_on_ctrl_c() {
	serves_pages_and_stops_on("INT");
}

#[test]
fn the_server_stops_on_sigterm() {
	serves_pages_and_stops_on("TERM");
}

/// Starts the binary, fetches a page, sends it `signal`, and requires a
/// clean exit within ten seconds.
fn serves_pages_and_stops_on(signal: &str) {
	let site_root = std::env::temp_dir().join(format!(
		"islands_example_binary_{signal}_{}",
		std::process::id()
	));
	std::fs::create_dir_all(&site_root).expect("site root");
	// A free port: bound, read, and released for the server to take.
	let address = TcpListener::bind("127.0.0.1:0")
		.and_then(|listener| listener.local_addr())
		.expect("a free port");

	let mut server = Command::new(env!("CARGO_BIN_EXE_islands_example"))
		.env("LEPTOS_OUTPUT_NAME", "islands_example")
		.env("LEPTOS_SITE_ROOT", &site_root)
		.env("LEPTOS_SITE_ADDR", address.to_string())
		.stdout(Stdio::piped())
		.spawn()
		.expect("start the server");

	let mut ready = String::new();
	BufReader::new(server.stdout.take().expect("stdout"))
		.read_line(&mut ready)
		.expect("the server reports its address");
	assert_eq!(ready.trim(), format!("listening on http://{address}"));

	let mut stream = TcpStream::connect(address).expect("connect");
	stream
		.write_all(b"GET /lab HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
		.expect("request");
	let mut response = String::new();
	stream.read_to_string(&mut response).expect("response");
	assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");
	assert!(
		response.contains("This island loaded on demand."),
		"{response}"
	);

	let signalled = Command::new("kill")
		.args([format!("-{signal}"), server.id().to_string()])
		.status()
		.expect("run kill");
	assert!(signalled.success());
	let deadline = Instant::now() + Duration::from_secs(10);
	let status = loop {
		if let Some(status) = server.try_wait().expect("poll the server") {
			break status;
		}
		assert!(Instant::now() < deadline, "the server ignored SIG{signal}");
		std::thread::sleep(Duration::from_millis(20));
	};
	let _ = std::fs::remove_dir_all(&site_root);
	assert!(
		status.success(),
		"a graceful shutdown exits cleanly: {status}"
	);
}
