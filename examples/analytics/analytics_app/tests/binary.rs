//! The server binary as a deployment runs it: configured through the
//! `LEPTOS_*` environment, answering the `/healthz` and `/readyz` probes
//! every generated health check calls, and draining on the SIGTERM a
//! container runtime sends.

#![cfg(unix)]

use std::io::BufRead;
use std::io::BufReader;
use std::io::Read;
use std::io::Write;
use std::net::SocketAddr;
use std::net::TcpListener;
use std::net::TcpStream;
use std::process::Command;
use std::process::Stdio;
use std::time::Duration;
use std::time::Instant;

#[test]
fn the_server_answers_its_probes_and_drains_on_sigterm() {
	let site_root = tempfile_dir();
	// A free port: bound, read, and released for the server to take.
	let address = TcpListener::bind("127.0.0.1:0")
		.and_then(|listener| listener.local_addr())
		.expect("a free port");
	let mut server = Command::new(env!("CARGO_BIN_EXE_analytics_app"))
		.env("LEPTOS_OUTPUT_NAME", "analytics_app")
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

	for probe in ["/healthz", "/readyz"] {
		let response = get(address, probe);
		assert!(
			response.starts_with("HTTP/1.1 200 OK"),
			"{probe}: {response}"
		);
	}

	let signalled = Command::new("kill")
		.args(["-TERM", &server.id().to_string()])
		.status()
		.expect("run kill");
	assert!(signalled.success());

	// While it drains, it still serves, but tells the load balancer it's
	// going away.
	let draining = get(address, "/readyz");
	assert!(draining.starts_with("HTTP/1.1 503"), "draining: {draining}");

	// Five seconds of drain, then the requests in flight: well within the
	// 30 seconds every generated deployment gives it.
	let deadline = Instant::now() + Duration::from_secs(20);
	let status = loop {
		if let Some(status) = server.try_wait().expect("poll the server") {
			break status;
		}
		assert!(Instant::now() < deadline, "the server ignored SIGTERM");
		std::thread::sleep(Duration::from_millis(50));
	};
	let _ = std::fs::remove_dir_all(&site_root);
	assert!(
		status.success(),
		"a graceful shutdown exits cleanly: {status}"
	);
}

fn get(address: SocketAddr, path: &str) -> String {
	let mut stream = TcpStream::connect(address).expect("connect");
	write!(
		stream,
		"GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
	)
	.expect("request");
	let mut response = String::new();
	stream.read_to_string(&mut response).expect("response");
	response
}

fn tempfile_dir() -> std::path::PathBuf {
	let dir = std::env::temp_dir().join(format!("analytics_app_binary_{}", std::process::id()));
	std::fs::create_dir_all(&dir).expect("site root");
	dir
}
