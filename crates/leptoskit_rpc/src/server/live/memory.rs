//! A session over an in-memory pipe, for tests: the client's frames go
//! straight to the session, and its frames straight back, with no socket.
//!
//! Each connection is identified the way the WebSocket's upgrade would be,
//! from the headers the test set, so a test can sign in by setting a
//! cookie or by handing the client a bearer token.

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::PoisonError;

use axum::http::HeaderMap;
use axum::http::HeaderName;
use axum::http::HeaderValue;
use leptoskit_query::BoxFuture;
use tokio::sync::mpsc;
use tokio::sync::watch;

use super::INCOMING;
use super::Incoming;
use super::Live;
use super::OUTGOING;
use super::Outbound;
use super::session::Opened;
use super::socket::read;
use crate::client::live::Connection;
use crate::client::live::Transport;

/// Opens in-memory sessions on a [`Live`].
#[derive(Clone)]
pub(crate) struct Pipe {
	live: Live,
	headers: Arc<Mutex<HeaderMap>>,
	/// Bumped to drop every open connection; `true` while new ones fail.
	down: watch::Sender<(u64, bool)>,
}

impl Pipe {
	pub(crate) fn new(live: &Live) -> Self {
		Self {
			live: live.clone(),
			headers: Arc::new(Mutex::new(HeaderMap::new())),
			down: watch::channel((0, false)).0,
		}
	}

	/// Sets a header every new connection's request carries.
	pub(crate) fn header(&self, name: &str, value: Option<&str>) {
		let Ok(name) = HeaderName::try_from(name) else {
			return;
		};
		let mut headers = self.headers.lock().unwrap_or_else(PoisonError::into_inner);

		match value.and_then(|value| HeaderValue::try_from(value).ok()) {
			Some(value) => {
				headers.insert(name, value);
			}
			None => {
				headers.remove(name);
			}
		}
	}

	/// Drops every open connection, and with `offline`, fails new ones
	/// until it is called again without.
	pub(crate) fn outage(&self, offline: bool) {
		self.down.send_modify(|(epoch, down)| {
			*epoch += 1;
			*down = offline;
		});
	}

	/// Whether new connections fail.
	pub(crate) fn is_down(&self) -> bool {
		self.down.borrow().1
	}
}

impl Transport for Pipe {
	fn connect(&self) -> BoxFuture<'static, Result<Box<dyn Connection>, String>> {
		let pipe = self.clone();
		Box::pin(async move {
			if pipe.is_down() {
				return Err("the network is down".to_owned());
			}

			let mut request = http::Request::builder().uri("/live");

			for (name, value) in pipe
				.headers
				.lock()
				.unwrap_or_else(PoisonError::into_inner)
				.iter()
			{
				request = request.header(name, value);
			}

			let (parts, ()) = request
				.body(())
				.map_err(|error| error.to_string())?
				.into_parts();
			let identity = pipe
				.live
				.inner
				.identify
				.identify(&parts)
				.await
				.map_err(|error| error.message().to_owned())?;

			let number = pipe.live.next_number();
			let (out, outgoing) = mpsc::channel::<Outbound>(OUTGOING);
			let (frames, incoming) = mpsc::channel::<Incoming>(INCOMING);
			let live = pipe.live.clone();
			tokio::spawn(async move {
				live.serve(Opened {
					number,
					connection: format!("live-{number}"),
					identity,
					frames: incoming,
					out,
					resume_floor: None,
					caller: Arc::new(Mutex::new(None)),
				})
				.await;
			});

			let mut down = pipe.down.subscribe();
			down.mark_unchanged();
			Ok(Box::new(Memory {
				frames,
				outgoing: Arc::new(tokio::sync::Mutex::new(outgoing)),
				down: Arc::new(tokio::sync::Mutex::new(down)),
			}) as Box<dyn Connection>)
		})
	}
}

/// One in-memory connection.
struct Memory {
	frames: mpsc::Sender<Incoming>,
	outgoing: Arc<tokio::sync::Mutex<mpsc::Receiver<Outbound>>>,
	down: Arc<tokio::sync::Mutex<watch::Receiver<(u64, bool)>>>,
}

impl Connection for Memory {
	fn send(&self, text: String) -> BoxFuture<'static, Result<(), String>> {
		let frames = self.frames.clone();
		Box::pin(async move {
			frames
				.send(read(&text))
				.await
				.map_err(|_| "the session closed".to_owned())
		})
	}

	fn next(&self) -> BoxFuture<'static, Result<Option<String>, String>> {
		let outgoing = Arc::clone(&self.outgoing);
		let down = Arc::clone(&self.down);
		Box::pin(async move {
			let mut outgoing = outgoing.lock().await;
			let mut down = down.lock().await;

			loop {
				tokio::select! {
					outbound = outgoing.recv() => match outbound {
						Some(Outbound::Frame { text, .. }) => return Ok(Some(text)),
						Some(Outbound::Heartbeat) => {}
						None => return Ok(None),
					},
					// An outage drops the connection.
					_ = down.changed() => return Ok(None),
				}
			}
		})
	}
}
