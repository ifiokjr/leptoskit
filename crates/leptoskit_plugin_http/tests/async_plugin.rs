#![cfg(all(feature = "async-host", not(target_arch = "wasm32")))]
use std::time::Duration;

use leptoskit_bridge::Bytes;
use leptoskit_bridge::Error;
use leptoskit_bridge::MethodId;
use leptoskit_bridge::async_host::AsyncPlugin;
use leptoskit_bridge::async_host::CancellationToken;
use leptoskit_plugin_http::Request;
use leptoskit_plugin_http::from_config;
use leptoskit_plugin_http::protocol;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;
use tokio::net::TcpStream;

#[test]
fn cancelled_dns_keeps_resolver_admission_until_blocking_work_finishes() {
	use std::future::poll_fn;
	use std::task::Poll;

	let runtime = tokio::runtime::Builder::new_current_thread()
		.enable_all()
		.max_blocking_threads(1)
		.build()
		.unwrap();
	let (release, receiver) = std::sync::mpsc::channel();
	let (entered, ready) = std::sync::mpsc::channel();
	let blocker = runtime.spawn_blocking(move || {
		entered.send(()).unwrap();
		let _ = receiver.recv();
	});
	ready.recv_timeout(Duration::from_secs(5)).unwrap();
	runtime.block_on(async {
		let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
		let authority = format!("localhost:{}", listener.local_addr().unwrap().port());
		let origin = format!("http://{authority}");
		let plugin = from_config(&format!(r#"{{"origins":["{origin}"]}}"#)).unwrap();
		let request = protocol::encode_request(&Request::get(origin)).unwrap();
		for _ in 0..4 {
			let cancellation = CancellationToken::new();
			let mut call = plugin.invoke(MethodId(0), request.clone(), cancellation.clone());
			poll_fn(|context| {
				assert!(call.as_mut().poll(context).is_pending());
				Poll::Ready(())
			})
			.await;
			cancellation.cancel();
			assert!(matches!(call.await, Err(Error::Cancelled)));
		}
		assert!(matches!(
			plugin
				.invoke(MethodId(0), request.clone(), CancellationToken::new())
				.await,
			Err(Error::Busy)
		));
		release.send(()).unwrap();
		blocker.await.unwrap();
		// The single blocking worker drains earlier resolver closures first.
		tokio::task::spawn_blocking(|| ()).await.unwrap();
		let server = tokio::spawn(async move {
			let (mut stream, _) = listener.accept().await.unwrap();
			let mut bytes = Vec::new();
			while !bytes.ends_with(b"\r\n\r\n") {
				bytes.push(stream.read_u8().await.unwrap());
				assert!(bytes.len() < 16_384);
			}
			assert!(
				String::from_utf8(bytes)
					.unwrap()
					.to_ascii_lowercase()
					.contains(&format!("\r\nhost: {authority}\r\n"))
			);
			stream
				.write_all(b"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n")
				.await
				.unwrap();
		});
		let reply = tokio::time::timeout(
			Duration::from_secs(5),
			plugin.invoke(MethodId(0), request, CancellationToken::new()),
		)
		.await
		.unwrap()
		.unwrap();
		assert_eq!(
			protocol::decode_response(reply.bytes()).unwrap().status,
			204
		);
		plugin.shutdown().await;
		server.await.unwrap();
	});
}

async fn head(stream: &mut TcpStream) {
	let mut bytes = Vec::new();
	while !bytes.ends_with(b"\r\n\r\n") {
		bytes.push(stream.read_u8().await.unwrap());
		assert!(bytes.len() < 16_384);
	}
}
#[tokio::test]
async fn typed_rpc_preserves_status_headers_binary_and_rejects_denied_destinations() {
	let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
	let origin = format!("http://{}", listener.local_addr().unwrap());
	let server = tokio::spawn(async move {
		let (mut stream, _) = listener.accept().await.unwrap();
		head(&mut stream).await;
		stream.write_all(b"HTTP/1.1 422 Unprocessable Content\r\nX-Test: yes\r\nContent-Length: 3\r\nConnection: close\r\n\r\n\x00\xff\x80").await.unwrap();
	});
	let plugin = from_config(&format!(r#"{{"origins":["{origin}"]}}"#)).unwrap();
	let request = protocol::encode_request(&Request::get(origin)).unwrap();
	let response = plugin
		.invoke(MethodId(0), request, CancellationToken::new())
		.await
		.unwrap();
	let response = protocol::decode_response(response.bytes()).unwrap();
	assert_eq!(response.status, 422);
	assert_eq!(response.body, [0, 255, 128]);
	assert!(
		response
			.headers
			.iter()
			.any(|h| h.name == "x-test" && h.value == b"yes")
	);
	server.await.unwrap();
	let denied = protocol::encode_request(&Request::get("http://127.0.0.1:1")).unwrap();
	assert!(matches!(
		plugin
			.invoke(MethodId(0), denied, CancellationToken::new())
			.await,
		Err(Error::PermissionDenied)
	));
	assert!(matches!(
		plugin
			.invoke(
				MethodId(0),
				Bytes::from_static(&[0, 1]),
				CancellationToken::new()
			)
			.await,
		Err(Error::InvalidArgument)
	));
}
#[tokio::test]
async fn cancellation_waits_until_inflight_transport_is_dropped() {
	let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
	let origin = format!("http://{}", listener.local_addr().unwrap());
	let (ready, receive_ready) = tokio::sync::oneshot::channel();
	let server = tokio::spawn(async move {
		let (mut stream, _) = listener.accept().await.unwrap();
		head(&mut stream).await;
		ready.send(()).unwrap();
		let result = tokio::time::timeout(Duration::from_secs(2), stream.read_u8())
			.await
			.unwrap();
		assert!(result.is_err_and(|e| {
			matches!(
				e.kind(),
				std::io::ErrorKind::UnexpectedEof
					| std::io::ErrorKind::ConnectionReset
					| std::io::ErrorKind::ConnectionAborted
			)
		}));
	});
	let plugin = from_config(&format!(r#"{{"origins":["{origin}"]}}"#)).unwrap();
	let token = CancellationToken::new();
	let cancellation = token.clone();
	let request = protocol::encode_request(&Request::get(origin)).unwrap();
	let operation = tokio::spawn(async move { plugin.invoke(MethodId(0), request, token).await });
	receive_ready.await.unwrap();
	cancellation.cancel();
	assert!(matches!(operation.await.unwrap(), Err(Error::Cancelled)));
	server.await.unwrap();
}
#[test]
fn configuration_and_frames_cannot_bypass_host_bounds() {
	for options in [
		"{}",
		r#"{"origins":["http://localhost"],"allow_all":true}"#,
		r#"{"origins":["http://localhost"],"max_body_bytes":8388608}"#,
	] {
		assert!(from_config(options).is_err());
	}
	let bytes = protocol::encode_request(&Request::get("http://localhost")).unwrap();
	for len in 0..bytes.len() {
		assert!(protocol::decode_request(&bytes[..len]).is_err());
	}
	let mut trailing = bytes.to_vec();
	trailing.push(0);
	assert!(protocol::decode_request(&trailing).is_err());
}
