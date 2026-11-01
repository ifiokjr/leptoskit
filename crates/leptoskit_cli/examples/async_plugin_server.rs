//! Loopback HTTP/WebSocket fixtures for `async_plugin_suite`.
//! Run this server before using the generated app's Check files action.

use std::net::SocketAddr;
use std::time::Duration;

use anyhow::Result;
use axum::Router;
use axum::extract::ws::Message;
use axum::extract::ws::WebSocketUpgrade;
use axum::http::StatusCode;
use axum::http::header;
use axum::response::IntoResponse;
use axum::routing::get;
use futures::SinkExt;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
	let address: SocketAddr = std::env::args()
		.nth(1)
		.unwrap_or_else(|| "127.0.0.1:48931".into())
		.parse()?;
	anyhow::ensure!(
		address.ip().is_loopback(),
		"fixture server must bind loopback"
	);
	let listener = tokio::net::TcpListener::bind(address).await?;
	println!("async plugin fixtures at http://{}", listener.local_addr()?);
	let router = Router::new()
		.route("/bytes", get(|| async { vec![0_u8, 127, 255] }))
		.route("/large", get(|| async { vec![0_u8; 8193] }))
		.route(
			"/redirect",
			get(|| {
				async {
					(
						StatusCode::FOUND,
						[(header::LOCATION, "http://denied.invalid/bytes")],
					)
				}
			}),
		)
		.route(
			"/slow",
			get(|| {
				async {
					tokio::time::sleep(Duration::from_secs(5)).await;
					"late"
				}
			}),
		)
		.route("/echo", get(echo));
	axum::serve(listener, router).await?;
	Ok(())
}

async fn echo(upgrade: WebSocketUpgrade) -> impl IntoResponse {
	upgrade.max_message_size(8192).on_upgrade(|mut socket| {
		async move {
			while let Some(message) = socket.recv().await {
				match message {
					Ok(message @ (Message::Text(_) | Message::Binary(_))) => {
						if socket.send(message).await.is_err() {
							break;
						}
					}
					Ok(Message::Close(_)) => {
						if let Err(error) = socket.flush().await {
							eprintln!("fixture WebSocket close failed: {error}");
						}

						break;
					}
					Err(_) => break,
					Ok(_) => {}
				}
			}
		}
	})
}
