#![cfg(not(target_arch = "wasm32"))]

use std::time::Duration;

use leptoskit_plugin_http::Client;
use leptoskit_plugin_http::Config;
use leptoskit_plugin_http::Error;
use leptoskit_plugin_http::Header;
use leptoskit_plugin_http::Method;
use leptoskit_plugin_http::Request;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;
use tokio::net::TcpStream;

async fn listener() -> (TcpListener, String) {
	let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
	let origin = format!("http://{}", listener.local_addr().unwrap());
	(listener, origin)
}
async fn request_head(stream: &mut TcpStream) -> Vec<u8> {
	let mut request = Vec::new();

	while !request.ends_with(b"\r\n\r\n") {
		let byte = stream.read_u8().await.unwrap();
		request.push(byte);
		assert!(request.len() < 16 * 1024);
	}

	request
}

async fn assert_transport_closed(stream: &mut TcpStream) {
	let mut buffer = [0; 1];
	let result = tokio::time::timeout(Duration::from_secs(2), stream.read(&mut buffer))
		.await
		.expect("cancelled request retained its socket");
	// Cancellation with unread response bytes may reset TCP instead of sending
	// FIN.
	match result {
		Ok(0) => {}
		Err(error)
			if matches!(
				error.kind(),
				std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::ConnectionAborted
			) => {}
		result => panic!("expected a closed transport, received {result:?}"),
	}
}

#[tokio::test]
async fn binary_bodies_headers_and_unsuccessful_status_are_preserved() {
	let (listener, origin) = listener().await;
	let server = tokio::spawn(async move {
		let (mut stream, _) = listener.accept().await.unwrap();
		let header = request_head(&mut stream).await;
		assert!(header.starts_with(b"POST /echo HTTP/1.1\r\n"));
		assert!(
			String::from_utf8_lossy(&header)
				.to_ascii_lowercase()
				.contains("x-input: binary\r\n")
		);
		let mut input = [0; 4];
		stream.read_exact(&mut input).await.unwrap();
		assert_eq!(input, [0, 255, 128, 42]);
		stream.write_all(b"HTTP/1.1 422 Unprocessable Content\r\nContent-Length: 4\r\nX-Reply: value\r\nConnection: close\r\n\r\n\x00\xff\x80\x2a").await.unwrap();
	});
	let client = Client::new(Config::new([&origin]).unwrap()).unwrap();
	let mut request = Request::get(format!("{origin}/echo"));
	request.method = Method::Post;
	request.headers.push(Header {
		name: "X-Input".into(),
		value: b"binary".to_vec(),
	});
	request.body = vec![0, 255, 128, 42];
	let response = client.execute(request).await.unwrap();

	assert_eq!(response.status, 422);
	assert_eq!(response.body, [0, 255, 128, 42]);
	assert!(
		response
			.headers
			.iter()
			.any(|header| header.name == "x-reply" && header.value == b"value")
	);
	server.await.unwrap();
}

#[tokio::test]
async fn chunked_body_cannot_bypass_the_size_limit() {
	let (listener, origin) = listener().await;
	let server = tokio::spawn(async move {
		let (mut stream, _) = listener.accept().await.unwrap();
		request_head(&mut stream).await;
		stream.write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n3\r\nabc\r\n3\r\ndef\r\n0\r\n\r\n").await.unwrap();
	});
	let mut config = Config::new([&origin]).unwrap();
	config.max_body_bytes = 4;
	let result = Client::new(config)
		.unwrap()
		.execute(Request::get(&origin))
		.await;
	assert!(matches!(result, Err(Error::TooLarge)));
	server.await.unwrap();
}

#[tokio::test]
async fn redirects_are_returned_without_contacting_the_target() {
	let (listener, origin) = listener().await;
	let (forbidden, destination) = self::listener().await;
	let server = tokio::spawn(async move {
		let (mut stream, _) = listener.accept().await.unwrap();
		request_head(&mut stream).await;
		stream
			.write_all(
				format!(
					"HTTP/1.1 302 Found\r\nLocation: {destination}/private\r\nContent-Length: \
					 0\r\nConnection: close\r\n\r\n"
				)
				.as_bytes(),
			)
			.await
			.unwrap();
	});
	let response = Client::new(Config::new([&origin]).unwrap())
		.unwrap()
		.execute(Request::get(origin))
		.await
		.unwrap();
	assert_eq!(response.status, 302);
	assert!(
		tokio::time::timeout(Duration::from_millis(50), forbidden.accept())
			.await
			.is_err()
	);
	server.await.unwrap();
}

#[tokio::test]
async fn total_timeout_covers_a_stalled_response_body() {
	let (listener, origin) = listener().await;
	let server = tokio::spawn(async move {
		let (mut stream, _) = listener.accept().await.unwrap();
		request_head(&mut stream).await;
		stream
			.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nx")
			.await
			.unwrap();
		assert_transport_closed(&mut stream).await;
	});
	let mut config = Config::new([&origin]).unwrap();
	config.timeout = Duration::from_millis(100);
	let result = Client::new(config)
		.unwrap()
		.execute(Request::get(origin))
		.await;
	assert!(matches!(result, Err(Error::Timeout)));
	server.await.unwrap();
}

#[tokio::test]
async fn dropping_in_flight_request_closes_the_response_stream() {
	let (listener, origin) = listener().await;
	let (ready, received) = tokio::sync::oneshot::channel();
	let server = tokio::spawn(async move {
		let (mut stream, _) = listener.accept().await.unwrap();
		request_head(&mut stream).await;
		stream
			.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\nx")
			.await
			.unwrap();
		ready.send(()).unwrap();
		assert_transport_closed(&mut stream).await;
	});
	let client = Client::new(Config::new([&origin]).unwrap()).unwrap();
	let pending = tokio::spawn(async move { client.execute(Request::get(origin)).await });
	received.await.unwrap();
	pending.abort();
	assert!(pending.await.unwrap_err().is_cancelled());

	server.await.unwrap();
}
