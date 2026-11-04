//! `GET /live`: a session over a WebSocket. The socket's text messages are
//! client frames, and every server frame is one text message back.

use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use axum::extract::FromRequestParts;
use axum::extract::State;
use axum::extract::WebSocketUpgrade;
use axum::extract::ws::Message;
use axum::extract::ws::WebSocket;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::response::Response;
use futures::SinkExt;
use futures::StreamExt;
use tokio::sync::mpsc;

use super::INCOMING;
use super::Identity;
use super::Incoming;
use super::Live;
use super::OUTGOING;
use super::Outbound;
use super::session::Opened;
use crate::live::ClientFrame;
use crate::live::MAX_FRAME_BYTES;
use crate::live::SUBPROTOCOL;
use crate::live::WireError;

/// How long a closed socket keeps reading for the client's close before
/// it is dropped anyway.
const LINGER: Duration = Duration::from_secs(5);

pub(super) async fn upgrade(State(live): State<Live>, request: axum::extract::Request) -> Response {
	let (mut parts, _body) = request.into_parts();

	if !live.origin_ok(&parts.headers) {
		return StatusCode::FORBIDDEN.into_response();
	}

	let identity = match live.inner.identify.identify(&parts).await {
		Ok(identity) => identity,
		Err(error) => return crate::server::http::failure(&error),
	};

	let websocket = match WebSocketUpgrade::from_request_parts(&mut parts, &()).await {
		Ok(websocket) => websocket,
		Err(rejection) => return rejection.into_response(),
	};

	websocket
		.protocols([SUBPROTOCOL])
		.on_upgrade(move |socket| async move { live.socket(socket, identity).await })
}

impl Live {
	async fn socket(self, socket: WebSocket, identity: Identity) {
		let number = self.next_number();
		let (out, mut outgoing) = mpsc::channel::<Outbound>(OUTGOING);
		let (frames, incoming) = mpsc::channel::<Incoming>(INCOMING);
		let (mut sink, mut stream) = socket.split();

		let writer = tokio::spawn(async move {
			while let Some(outbound) = outgoing.recv().await {
				let message = match outbound {
					Outbound::Frame { text, .. } => Message::Text(text.into()),
					Outbound::Heartbeat => Message::Ping(Vec::new().into()),
				};

				if sink.send(message).await.is_err() {
					break;
				}
			}

			let _ = sink.close().await;
		});

		let reader = tokio::spawn(async move {
			let mut frames = Some(frames);

			while let Some(Ok(message)) = stream.next().await {
				// Once the session has ended, what the client still sends is
				// read and dropped until its close arrives (see [`LINGER`]).
				let Some(open) = &frames else {
					if matches!(message, Message::Close(_)) {
						break;
					}

					continue;
				};
				let incoming = match message {
					Message::Text(text) => read(text.as_str()),
					Message::Binary(bytes) => {
						match std::str::from_utf8(&bytes) {
							Ok(text) => read(text),
							Err(_) => continue,
						}
					}
					Message::Close(_) => break,
					Message::Ping(_) | Message::Pong(_) => continue,
				};

				if open.send(incoming).await.is_err() {
					frames = None;
				}
			}
		});

		self.serve(Opened {
			number,
			connection: format!("live-{number}"),
			identity,
			frames: incoming,
			out,
			resume_floor: None,
			caller: Arc::new(Mutex::new(None)),
		})
		.await;

		// The writer ends with the server's close. Dropping the socket while
		// the client's frames sit unread makes the kernel reset the
		// connection, and a reset discards what the client hasn't read yet,
		// on Windows its last frames and the close with them. So the reader
		// keeps draining until the client answers the close, or for
		// [`LINGER`] if it never does.
		let _ = writer.await;
		let draining = reader.abort_handle();

		if tokio::time::timeout(LINGER, reader).await.is_err() {
			draining.abort();
		}
	}
}

/// One text message as a client frame, or why it isn't one.
pub(super) fn read(text: &str) -> Incoming {
	if text.len() > MAX_FRAME_BYTES {
		return Incoming::Invalid(WireError::invalid(format!(
			"the frame is larger than {MAX_FRAME_BYTES} bytes"
		)));
	}

	match serde_json::from_str::<ClientFrame>(text) {
		Ok(frame) => Incoming::Frame(frame),
		Err(error) => {
			Incoming::Invalid(WireError::invalid(format!(
				"the frame doesn't parse: {error}"
			)))
		}
	}
}
