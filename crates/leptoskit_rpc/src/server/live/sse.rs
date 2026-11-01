//! A session over server-sent events, for networks a WebSocket can't get
//! through: `GET /live/events` streams the server's frames, and the client
//! posts its own to `POST /live/frames?connection=<id>`.
//!
//! The stream is authenticated like the socket's upgrade: the `Origin`
//! check and the session cookie, or a bearer token sent in an `auth` frame.
//! Each post is checked the same way and must come from the caller the
//! connection belongs to, or start with an `auth` frame. The connection's
//! name carries a random secret, so only the stream's reader can post to
//! it.

use std::convert::Infallible;
use std::fmt::Write;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::PoisonError;

use axum::body::Body;
use axum::extract::State;
use axum::http::HeaderValue;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::response::Response;
use axum::response::sse::Event;
use axum::response::sse::Sse;
use tokio::sync::mpsc;

use super::INCOMING;
use super::Incoming;
use super::Live;
use super::OUTGOING;
use super::Outbound;
use super::session::CallerSlot;
use super::session::Opened;
use crate::live::ClientFrame;
use crate::live::MAX_FRAME_BYTES;

/// How a post finds its connection.
pub(super) struct Handle {
	frames: mpsc::Sender<Incoming>,
	caller: CallerSlot,
}

/// `GET /live/events`: opens a session and streams its frames.
pub(super) async fn events(State(live): State<Live>, request: axum::extract::Request) -> Response {
	let (parts, _body) = request.into_parts();

	if !live.origin_ok(&parts.headers) {
		return StatusCode::FORBIDDEN.into_response();
	}

	let identity = match live.inner.identify.identify(&parts).await {
		Ok(identity) => identity,
		Err(error) => return crate::server::http::failure(&error),
	};
	let Some(secret) = secret() else {
		return StatusCode::SERVICE_UNAVAILABLE.into_response();
	};
	let resume_floor = parts
		.headers
		.get("last-event-id")
		.and_then(|value| value.to_str().ok())
		.and_then(|value| value.trim().parse::<u64>().ok());

	let number = live.next_number();
	let connection = format!("live-{number}.{secret}");
	let (out, mut outgoing) = mpsc::channel::<Outbound>(OUTGOING);
	let (frames, incoming) = mpsc::channel::<Incoming>(INCOMING);
	let caller: CallerSlot = Arc::new(Mutex::new(identity.caller.clone()));
	live.event_streams().insert(
		connection.clone(),
		Handle {
			frames,
			caller: Arc::clone(&caller),
		},
	);

	let session = live.clone();
	let name = connection.clone();
	tokio::spawn(async move {
		session
			.serve(Opened {
				number,
				connection: name.clone(),
				identity,
				frames: incoming,
				out,
				resume_floor,
				caller,
			})
			.await;
		session.event_streams().remove(&name);
	});

	let stream = futures::stream::poll_fn(move |cx| {
		outgoing.poll_recv(cx).map(|outbound| {
			outbound.map(|outbound| {
				Ok::<Event, Infallible>(match outbound {
					Outbound::Frame { text, version } => {
						let event = Event::default().data(text);
						match version {
							Some(version) => event.id(version.to_string()),
							None => event,
						}
					}
					Outbound::Heartbeat => Event::default().comment("heartbeat"),
				})
			})
		})
	});

	let mut response = Sse::new(stream).into_response();
	// Proxies that buffer responses would hold every frame back.
	response
		.headers_mut()
		.insert("x-accel-buffering", HeaderValue::from_static("no"));
	response
}

/// `POST /live/frames?connection=<id>`: hands a JSON array of client frames
/// to the connection's session. Their answers arrive on the stream.
pub(super) async fn frames(State(live): State<Live>, request: axum::extract::Request) -> Response {
	let (parts, body) = request.into_parts();

	if !live.origin_ok(&parts.headers) {
		return StatusCode::FORBIDDEN.into_response();
	}

	let Some(connection) = parts.uri.query().and_then(|query| {
		query
			.split('&')
			.find_map(|pair| pair.strip_prefix("connection=").map(str::to_owned))
	}) else {
		return refuse(
			StatusCode::BAD_REQUEST,
			"say which connection with `?connection=<id>`, from its `welcome`",
		);
	};

	let Ok(bytes) = axum::body::to_bytes(body, MAX_FRAME_BYTES).await else {
		return refuse(
			StatusCode::PAYLOAD_TOO_LARGE,
			&format!("post at most {MAX_FRAME_BYTES} bytes of frames at a time"),
		);
	};
	let frames: Vec<ClientFrame> = match serde_json::from_slice(&bytes) {
		Ok(frames) => frames,
		Err(error) => {
			return refuse(
				StatusCode::BAD_REQUEST,
				&format!("send a JSON array of frames: {error}"),
			);
		}
	};

	let identity = match live.inner.identify.identify(&parts).await {
		Ok(identity) => identity,
		Err(error) => return crate::server::http::failure(&error),
	};

	let (sender, bound) = {
		let streams = live.event_streams();
		let Some(handle) = streams.get(&connection) else {
			return refuse(
				StatusCode::NOT_FOUND,
				"that live connection has closed; open the event stream again",
			);
		};
		let bound = handle
			.caller
			.lock()
			.unwrap_or_else(PoisonError::into_inner)
			.clone();
		(handle.frames.clone(), bound)
	};

	let authenticates = matches!(frames.first(), Some(ClientFrame::Auth { .. }));

	if identity.caller != bound && !authenticates {
		return refuse(
			StatusCode::FORBIDDEN,
			"that live connection belongs to someone else",
		);
	}

	for frame in frames {
		if sender.send(Incoming::Frame(frame)).await.is_err() {
			return refuse(
				StatusCode::NOT_FOUND,
				"that live connection has closed; open the event stream again",
			);
		}
	}

	StatusCode::ACCEPTED.into_response()
}

fn refuse(status: StatusCode, message: &str) -> Response {
	let mut response = Response::new(Body::from(message.to_owned()));
	*response.status_mut() = status;
	response
}

/// 128 random bits, as hex.
fn secret() -> Option<String> {
	let mut bytes = [0_u8; 16];
	getrandom::fill(&mut bytes).ok()?;
	Some(
		bytes
			.iter()
			.fold(String::with_capacity(32), |mut hex, byte| {
				let _ = write!(hex, "{byte:02x}");
				hex
			}),
	)
}
