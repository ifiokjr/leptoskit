//! The development server, driven over real sockets.

use std::fs;
use std::io::Read;
use std::io::Write;
use std::net::SocketAddr;
use std::net::TcpStream;
use std::time::Duration;
use std::time::Instant;

use leptoskit_docs::Error;
use leptoskit_docs::Site;

/// Sends `request` and returns the status line and body.
fn send(address: SocketAddr, request: &str) -> (String, String) {
	let mut stream = TcpStream::connect(address).unwrap();
	stream
		.set_read_timeout(Some(Duration::from_secs(40)))
		.unwrap();
	stream.write_all(request.as_bytes()).unwrap();

	let mut response = Vec::new();
	stream.read_to_end(&mut response).unwrap();
	let response = String::from_utf8_lossy(&response);
	let (head, body) = response.split_once("\r\n\r\n").unwrap_or((&response, ""));
	(
		head.lines().next().unwrap_or_default().to_owned(),
		body.to_owned(),
	)
}

fn get(address: SocketAddr, path: &str) -> (String, String) {
	send(
		address,
		&format!("GET {path} HTTP/1.1\r\nHost: localhost\r\n\r\n"),
	)
}

#[test]
fn serves_rebuilds_and_reloads() {
	let root = tempfile::tempdir().unwrap();
	fs::write(
		root.path().join("SUMMARY.md"),
		"[Home](readme.md)\n- [Guide](guide/readme.md)\n",
	)
	.unwrap();
	fs::write(root.path().join("readme.md"), "# Home\n\nFirst draft.\n").unwrap();
	fs::create_dir(root.path().join("guide")).unwrap();
	fs::write(root.path().join("guide/readme.md"), "# Guide\n").unwrap();
	fs::write(root.path().join("photo.png"), [0x89, b'P', b'N', b'G']).unwrap();
	let out = root.path().join("dist");

	let server = Site::new(root.path()).serve(&out, "127.0.0.1:0").unwrap();
	let address = server.address();

	let (status, home) = get(address, "/");
	assert_eq!(status, "HTTP/1.1 200 OK");
	assert!(home.contains("First draft."), "{home}");
	assert!(
		home.contains("data-live-reload=\"\""),
		"served pages reload themselves"
	);

	assert!(get(address, "/guide/").1.contains("<h1 id=\"guide\">"));
	assert!(
		get(address, "/_leptoskit/site.css")
			.1
			.contains("--lk-ember")
	);
	assert_eq!(get(address, "/photo.png").0, "HTTP/1.1 200 OK");

	let (status, body) = get(address, "/nowhere.html");
	assert_eq!(status, "HTTP/1.1 404 Not Found");
	assert!(body.contains("Page not found"));

	assert_eq!(get(address, "/../secret").0, "HTTP/1.1 400 Bad Request");
	assert_eq!(
		send(address, "POST / HTTP/1.1\r\n\r\n").0,
		"HTTP/1.1 405 Method Not Allowed"
	);
	assert_eq!(send(address, "\r\n").0, "HTTP/1.1 400 Bad Request");

	let (_, generation) = get(address, "/_leptoskit/live?since=0");
	let generation: u64 = generation.parse().unwrap();
	assert!(generation >= 1, "the first build counts");

	// A broken edit keeps the last good build up and still wakes the page.
	fs::write(root.path().join("readme.md"), "# Home\n\n<Unclosed>\n").unwrap();
	let (_, broken) = get(address, &format!("/_leptoskit/live?since={generation}"));
	assert!(broken.parse::<u64>().unwrap() > generation);
	assert!(get(address, "/").1.contains("First draft."));

	fs::write(root.path().join("readme.md"), "# Home\n\nSecond draft.\n").unwrap();
	let deadline = Instant::now() + Duration::from_secs(30);

	while !get(address, "/").1.contains("Second draft.") {
		assert!(Instant::now() < deadline, "the edit was never rebuilt");
		std::thread::sleep(Duration::from_millis(50));
	}
}

#[test]
fn fails_to_serve_on_a_taken_address() {
	let taken = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
	let root = tempfile::tempdir().unwrap();

	let error = Site::new(root.path())
		.serve(root.path().join("dist"), taken.local_addr().unwrap())
		.err()
		.unwrap();

	assert!(matches!(error, Error::Listen(_)), "{error}");
	assert!(error.to_string().starts_with("could not serve the docs: "));
}

#[test]
fn fails_to_watch_a_missing_book() {
	let root = tempfile::tempdir().unwrap();
	let missing = root.path().join("missing");

	let error = Site::new(&missing)
		.serve(root.path().join("dist"), "127.0.0.1:0")
		.err()
		.unwrap();

	assert!(matches!(error, Error::Watch { .. }), "{error}");
}
