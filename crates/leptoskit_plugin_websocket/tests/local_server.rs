#![cfg(not(target_arch = "wasm32"))]

use std::time::Duration;

use futures::SinkExt;
use futures::StreamExt;
use leptoskit_plugin_websocket::Client;
use leptoskit_plugin_websocket::Config;
use leptoskit_plugin_websocket::Error;
use leptoskit_plugin_websocket::Message;
use tokio::io::AsyncReadExt;
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite;

async fn listener() -> (TcpListener, String) {
	let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
	let url = format!("ws://{}", listener.local_addr().unwrap());
	(listener, url)
}

#[tokio::test]
async fn text_binary_and_graceful_close_round_trip() {
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
	let client = Client::new(Config::new([&url]).unwrap()).unwrap();
	let mut socket = client.connect(&url, &[]).await.unwrap();
	for message in [
		Message::Text("hello".into()),
		Message::Binary(vec![0, 255, 128]),
	] {
		socket.send(message.clone()).await.unwrap();
		assert_eq!(socket.receive().await.unwrap(), Some(message));
	}
	socket.close(1000, "done").await.unwrap();
	assert_eq!(socket.receive().await.unwrap(), None);
	assert!(matches!(
		socket.send(Message::Text("late".into())).await,
		Err(Error::Closed)
	));
	server.await.unwrap();
}

#[tokio::test]
async fn oversized_incoming_message_terminates_the_connection() {
	let (listener, url) = listener().await;
	let server = tokio::spawn(async move {
		let (stream, _) = listener.accept().await.unwrap();
		let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
		socket
			.send(tungstenite::Message::Binary(vec![0; 9].into()))
			.await
			.unwrap();
	});
	let mut config = Config::new([&url]).unwrap();
	config.max_message_bytes = 8;
	let mut socket = Client::new(config)
		.unwrap()
		.connect(&url, &[])
		.await
		.unwrap();
	assert!(matches!(socket.receive().await, Err(Error::TooLarge)));
	assert!(socket.receive().await.unwrap().is_none());
	server.await.unwrap();
}

#[tokio::test]
async fn oversized_outgoing_message_is_rejected_without_losing_the_connection() {
	let (listener, url) = listener().await;
	let server = tokio::spawn(async move {
		let (stream, _) = listener.accept().await.unwrap();
		let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
		let message = socket.next().await.unwrap().unwrap();
		assert_eq!(message, tungstenite::Message::Text("ok".into()));
		socket.send(message).await.unwrap();
	});
	let mut config = Config::new([&url]).unwrap();
	config.max_message_bytes = 8;
	let mut socket = Client::new(config)
		.unwrap()
		.connect(&url, &[])
		.await
		.unwrap();
	assert!(matches!(
		socket.send(Message::Binary(vec![0; 9])).await,
		Err(Error::TooLarge)
	));
	socket.send(Message::Text("ok".into())).await.unwrap();
	assert_eq!(
		socket.receive().await.unwrap(),
		Some(Message::Text("ok".into()))
	);
	server.await.unwrap();
}

#[tokio::test]
async fn handshake_timeout_releases_the_tcp_connection() {
	let (listener, url) = listener().await;
	let server = tokio::spawn(async move {
		let (mut stream, _) = listener.accept().await.unwrap();
		let mut request = Vec::new();
		tokio::time::timeout(Duration::from_secs(2), stream.read_to_end(&mut request))
			.await
			.unwrap()
			.unwrap();
		assert!(request.starts_with(b"GET / HTTP/1.1\r\n"));
	});
	let mut config = Config::new([&url]).unwrap();
	config.connect_timeout = Duration::from_millis(100);
	assert!(matches!(
		Client::new(config).unwrap().connect(&url, &[]).await,
		Err(Error::Timeout)
	));
	server.await.unwrap();
}

#[tokio::test]
async fn dropping_socket_releases_transport_without_a_background_receiver() {
	let (listener, url) = listener().await;
	let server = tokio::spawn(async move {
		let (stream, _) = listener.accept().await.unwrap();
		let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
		assert!(
			tokio::time::timeout(Duration::from_secs(2), socket.next())
				.await
				.unwrap()
				.unwrap()
				.is_err()
		);
	});
	let socket = Client::new(Config::new([&url]).unwrap())
		.unwrap()
		.connect(&url, &[])
		.await
		.unwrap();
	drop(socket);
	server.await.unwrap();
}

#[tokio::test]
async fn control_frames_are_serviced_and_peer_close_is_acknowledged() {
	let (listener, url) = listener().await;
	let server = tokio::spawn(async move {
		let (stream, _) = listener.accept().await.unwrap();
		let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
		socket
			.send(tungstenite::Message::Ping(vec![1, 2, 3].into()))
			.await
			.unwrap();
		assert_eq!(
			socket.next().await.unwrap().unwrap(),
			tungstenite::Message::Pong(vec![1, 2, 3].into())
		);
		socket.close(None).await.unwrap();
		assert!(matches!(
			socket.next().await,
			Some(Ok(tungstenite::Message::Close(_)))
		));
	});
	let mut socket = Client::new(Config::new([&url]).unwrap())
		.unwrap()
		.connect(&url, &[])
		.await
		.unwrap();
	assert!(
		tokio::time::timeout(Duration::from_secs(2), socket.receive())
			.await
			.unwrap()
			.unwrap()
			.is_none()
	);
	server.await.unwrap();
}

#[tokio::test]
async fn closing_after_the_server_closed_first_succeeds() {
	let (listener, url) = listener().await;
	let server = tokio::spawn(async move {
		let (stream, _) = listener.accept().await.unwrap();
		let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
		socket.close(None).await.unwrap();
	});
	let client = Client::new(Config::new([&url]).unwrap()).unwrap();
	let mut socket = client.connect(&url, &[]).await.unwrap();
	server.await.unwrap();
	socket.close(1000, "done").await.unwrap();
	assert_eq!(socket.receive().await.unwrap(), None);
}
