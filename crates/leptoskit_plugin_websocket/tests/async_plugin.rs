#![cfg(all(feature = "async-host", not(target_arch = "wasm32")))]
use std::sync::Arc;
use std::time::Duration;

use futures::SinkExt;
use futures::StreamExt;
use leptoskit_bridge::Bridge;
use leptoskit_bridge::Bytes;
use leptoskit_bridge::Error;
use leptoskit_bridge::MethodId;
use leptoskit_bridge::Reply;
use leptoskit_bridge::async_host::AsyncPlugin;
use leptoskit_bridge::async_host::CancellationToken;
use leptoskit_plugin_websocket::Message;
use leptoskit_plugin_websocket::WebSocketPlugin;
use leptoskit_plugin_websocket::from_config;
use leptoskit_plugin_websocket::protocol;
use leptoskit_plugin_websocket::protocol::ConnectionId;
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite;

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
		// Dropping the sender also unblocks cleanup if an assertion fails.
		let _ = receiver.recv();
	});
	ready.recv_timeout(Duration::from_secs(5)).unwrap();
	runtime.block_on(async {
		let (listener, _) = listener().await;
		let authority = format!("localhost:{}", listener.local_addr().unwrap().port());
		let url = format!("ws://{authority}");
		let plugin = plugin(&url);
		for _ in 0..4 {
			let cancellation = CancellationToken::new();
			let mut call = plugin.invoke(
				MethodId(0),
				protocol::open(&url, &[]).unwrap(),
				cancellation.clone(),
			);
			poll_fn(|context| {
				assert!(call.as_mut().poll(context).is_pending());
				Poll::Ready(())
			})
			.await;
			cancellation.cancel();
			assert!(matches!(call.await, Err(Error::Cancelled)));
		}

		// All guest calls have completed cancellation, but their queued OS
		// resolver jobs must still occupy bounded admission.
		assert!(matches!(
			rpc(&plugin, 0, protocol::open(&url, &[]).unwrap()).await,
			Err(Error::Busy)
		));
		release.send(()).unwrap();
		blocker.await.unwrap();
		// One blocking thread processes the earlier DNS queue before this
		// barrier. Its completion proves the resolver leases can be reused.
		tokio::task::spawn_blocking(|| ()).await.unwrap();
		let server = tokio::spawn(async move {
			let (stream, _) = listener.accept().await.unwrap();
			#[expect(
				clippy::result_large_err,
				reason = "tungstenite's handshake callback fixes its error type"
			)]
			let socket = tokio_tungstenite::accept_hdr_async(
				stream,
				|request: &tungstenite::handshake::server::Request, response| {
					assert_eq!(request.headers()["host"], authority);
					Ok(response)
				},
			)
			.await
			.unwrap();
			drop(socket);
		});
		let reply = tokio::time::timeout(
			Duration::from_secs(5),
			rpc(&plugin, 0, protocol::open(&url, &[]).unwrap()),
		)
		.await
		.unwrap()
		.unwrap();
		drop(reply);
		plugin.shutdown().await;
		server.await.unwrap();
	});
}

async fn listener() -> (TcpListener, String) {
	let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
	let url = format!("ws://{}", listener.local_addr().unwrap());
	(listener, url)
}
fn plugin(url: &str) -> Arc<WebSocketPlugin> {
	Arc::new(from_config(&format!(r#"{{"origins":["{url}"],"max_connections":1}}"#)).unwrap())
}
fn consume(reply: Reply) -> Bytes {
	let mut bridge = Bridge::default();
	let id = bridge.register("ws", [MethodId(0)]).unwrap();
	bridge.grant(id, MethodId(0), true).unwrap();
	let call = bridge.submit(id, MethodId(0), Bytes::new()).unwrap();
	bridge.next_request().unwrap();
	bridge.complete_reply(call, Ok(reply)).unwrap();
	bridge.take_result(call).unwrap().unwrap().unwrap()
}
async fn open(plugin: &WebSocketPlugin, url: &str) -> ConnectionId {
	protocol::decode_id(&consume(
		plugin
			.invoke(
				MethodId(0),
				protocol::open(url, &[]).unwrap(),
				CancellationToken::new(),
			)
			.await
			.unwrap(),
	))
	.unwrap()
}
async fn rpc(plugin: &WebSocketPlugin, method: u32, bytes: Bytes) -> Result<Reply, Error> {
	plugin
		.invoke(MethodId(method), bytes, CancellationToken::new())
		.await
}

#[tokio::test]
async fn pending_receive_allows_sends_and_cancelled_waiter_can_be_replaced() {
	let (listener, url) = listener().await;
	let server = tokio::spawn(async move {
		let (stream, _) = listener.accept().await.unwrap();
		let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
		for _ in 0..2 {
			let message = socket.next().await.unwrap().unwrap();
			socket.send(message).await.unwrap();
		}

		assert!(matches!(
			socket.next().await,
			Some(Ok(tungstenite::Message::Close(_)))
		));
		socket.flush().await.unwrap();
	});
	let plugin = plugin(&url);
	let id = open(&plugin, &url).await;
	for message in [
		Message::Text("hello".into()),
		Message::Binary(vec![0, 255, 128]),
	] {
		let token = CancellationToken::new();
		let mut receive = plugin.invoke(MethodId(2), protocol::id(id), token.clone());
		assert!(futures::poll!(&mut receive).is_pending());
		assert!(matches!(
			rpc(&plugin, 2, protocol::id(id)).await,
			Err(Error::Busy)
		));
		token.cancel();
		assert!(matches!(receive.await, Err(Error::Cancelled)));
		let (received, sent) = tokio::join!(
			rpc(&plugin, 2, protocol::id(id)),
			rpc(&plugin, 1, protocol::send(id, &message).unwrap())
		);
		sent.unwrap();
		assert_eq!(
			protocol::decode_message(received.unwrap().bytes()).unwrap(),
			Some(message)
		);
	}
	rpc(&plugin, 3, protocol::close(id, 1000, "done").unwrap())
		.await
		.unwrap();
	server.await.unwrap();
	plugin.shutdown().await;
}

#[tokio::test]
async fn discarded_open_reply_and_shutdown_wait_for_actual_socket_release() {
	let (listener, url) = listener().await;
	let server = tokio::spawn(async move {
		let (stream, _) = listener.accept().await.unwrap();
		let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
		let next = tokio::time::timeout(Duration::from_secs(2), socket.next())
			.await
			.unwrap();
		assert!(!matches!(
			next,
			Some(Ok(
				tungstenite::Message::Text(_) | tungstenite::Message::Binary(_)
			))
		));
	});
	let plugin = plugin(&url);
	let reply = plugin
		.invoke(
			MethodId(0),
			protocol::open(&url, &[]).unwrap(),
			CancellationToken::new(),
		)
		.await
		.unwrap();
	assert!(matches!(
		plugin
			.invoke(
				MethodId(0),
				protocol::open(&url, &[]).unwrap(),
				CancellationToken::new()
			)
			.await,
		Err(Error::Busy)
	));
	drop(reply);
	plugin.shutdown().await;
	server.await.unwrap();
}

#[tokio::test]
async fn cancellation_of_open_permission_closes_committed_socket() {
	let (listener, url) = listener().await;
	let server = tokio::spawn(async move {
		let (stream, _) = listener.accept().await.unwrap();
		let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
		let next = tokio::time::timeout(Duration::from_secs(2), socket.next())
			.await
			.unwrap();
		assert!(!matches!(
			next,
			Some(Ok(
				tungstenite::Message::Text(_) | tungstenite::Message::Binary(_)
			))
		));
	});
	let plugin = plugin(&url);
	let permission = CancellationToken::new();
	let reply = plugin
		.invoke(
			MethodId(0),
			protocol::open(&url, &[]).unwrap(),
			permission.clone(),
		)
		.await
		.unwrap();
	consume(reply);
	permission.cancel();
	plugin.shutdown().await;

	server.await.unwrap();
}

#[tokio::test]
async fn scopes_malformed_frames_and_closed_plugin_prevent_network_access() {
	let (listener, url) = listener().await;
	let plugin = plugin(&url);
	assert!(matches!(
		rpc(&plugin, 0, protocol::open("ws://127.0.0.1:1", &[]).unwrap()).await,
		Err(Error::PermissionDenied)
	));
	assert!(matches!(
		rpc(&plugin, 0, Bytes::from_static(&[1, 2, 3])).await,
		Err(Error::InvalidArgument)
	));
	plugin.shutdown().await;
	assert!(matches!(
		rpc(&plugin, 0, protocol::open(&url, &[]).unwrap()).await,
		Err(Error::Cancelled)
	));
	assert!(
		tokio::time::timeout(Duration::from_millis(30), listener.accept())
			.await
			.is_err()
	);
}

#[tokio::test]
async fn completed_close_releases_connection_capacity_before_reopening() {
	let (listener, url) = listener().await;
	let server = tokio::spawn(async move {
		for _ in 0..2 {
			let (stream, _) = listener.accept().await.unwrap();
			let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
			assert!(matches!(
				socket.next().await,
				Some(Ok(tungstenite::Message::Close(_)))
			));
			socket.flush().await.unwrap();
		}
	});
	let plugin = plugin(&url);

	for _ in 0..2 {
		let id = open(&plugin, &url).await;
		rpc(&plugin, 3, protocol::close(id, 1000, "next").unwrap())
			.await
			.unwrap();
	}

	plugin.shutdown().await;
	server.await.unwrap();
}
