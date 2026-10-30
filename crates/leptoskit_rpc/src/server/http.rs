//! The one-shot HTTP transport: `POST /api/<name>` for every query and
//! mutation, and the same calls in process while the server renders.

use std::sync::Arc;
use std::sync::PoisonError;
use std::sync::RwLock;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::extract::Request;
use axum::extract::WebSocketUpgrade;
use axum::extract::ws::Message;
use axum::extract::ws::WebSocket;
use axum::http::HeaderValue;
use axum::http::StatusCode;
use axum::http::header;
use axum::response::IntoResponse;
use axum::response::Response;
use axum::routing::get;
use axum::routing::post;
use futures::SinkExt;
use futures::StreamExt;
use leptoskit_db::Db;
use serde_json::Value;

use super::Anonymous;
use super::Identify;
use super::Invocation;
use super::Registration;
use crate::Error;
use crate::Kind;
use crate::live::ClientFrame;
use crate::live::LiveConnection;
use crate::live::LiveHub;
use crate::live::ServerFrame;
use crate::live::Update;
use crate::live::UpdateOutcome;

/// The header a mutation's request ID travels in.
pub const IDEMPOTENCY_KEY: &str = "idempotency-key";

/// The largest request body a call may send: 2 MiB.
const BODY_LIMIT: usize = 2 * 1024 * 1024;

/// The app's operations, served: the database they run on, and how to tell
/// who is calling.
///
/// ```ignore
/// let rpc = rpc::server::Server::new(db.clone()).identify(auth.clone());
/// let app = Router::new().merge(rpc.router()).layer(auth.layer());
/// ```
#[derive(Clone)]
pub struct Server {
	inner: Arc<Inner>,
}

struct Inner {
	db: Db,
	identify: Arc<dyn Identify>,
	live: Arc<LiveHub>,
	polling: Arc<AtomicBool>,
}

impl Server {
	/// Operations on `db`, where every caller is anonymous until
	/// [`identify`](Self::identify) says otherwise.
	pub fn new(db: Db) -> Self {
		Self {
			inner: Arc::new(Inner {
				db: db.clone(),
				identify: Arc::new(Anonymous),
				live: Arc::new(LiveHub::new(db)),
				polling: Arc::new(AtomicBool::new(false)),
			}),
		}
	}

	/// Tells callers apart with `identify`, such as `leptoskit_auth`'s
	/// `Auth`.
	#[must_use]
	pub fn identify(self, identify: impl Identify) -> Self {
		Self {
			inner: Arc::new(Inner {
				db: self.inner.db.clone(),
				identify: Arc::new(identify),
				live: self.inner.live.clone(),
				polling: self.inner.polling.clone(),
			}),
		}
	}

	/// The database operations run on, before it acts for anyone.
	pub fn db(&self) -> &Db {
		&self.inner.db
	}

	/// Makes this the server that in-process calls use, such as
	/// `client::call` while a page renders. [`router`](Self::router) does it
	/// too.
	pub fn install(&self) {
		*INSTALLED.write().unwrap_or_else(PoisonError::into_inner) = Some(self.clone());
	}

	/// Starts the live hub's poll loop. [`router`](Self::router) calls it too,
	/// so a server that only uses [`router`](Self::router) needs nothing
	/// else. Calling it again does nothing.
	pub fn start(&self) {
		if self.inner.polling.swap(true, Ordering::AcqRel) {
			return;
		}

		let hub = self.inner.live.clone();
		tokio::spawn(async move {
			let mut tick = tokio::time::interval(Duration::from_millis(50));
			loop {
				tick.tick().await;
				let _ = hub.poll().await;
			}
		});
	}

	/// The live hub, for a test or a server that wires the socket itself.
	pub fn live(&self) -> &Arc<LiveHub> {
		&self.inner.live
	}

	/// `POST /api/<name>` for every registered query and mutation, relative
	/// to wherever the router is nested, and [installs](Self::install) this
	/// server for in-process calls.
	///
	/// A call sends its arguments as one JSON object, and gets back its
	/// result as JSON with status `200`, or a `Kind|message` error as
	/// `text/plain` with the status [`Error::status`] gives: the same as a
	/// `#[server]` function with the `Json` encoding, so every generated
	/// client speaks it. A mutation's `Idempotency-Key` header becomes its
	/// request ID.
	///
	/// # Panics
	///
	/// When two operations share a name; see
	/// [`registrations`](super::registrations).
	pub fn router<S>(&self) -> Router<S>
	where
		S: Clone + Send + Sync + 'static,
	{
		let registrations = super::registrations().unwrap_or_else(|error| panic!("{error}"));
		self.install();
		self.start();
		let live = self.inner.live.clone();
		let resolver = self.inner.identify.clone();
		let mut router = Router::new().route(
			"/live",
			get(move |upgrade: WebSocketUpgrade, request: Request| {
				let live = live.clone();
				let resolver = resolver.clone();
				async move {
					let (parts, _) = request.into_parts();
					match resolver.identify(&parts).await {
						Ok(identity) => {
							let (connection, frames) = live.connect(identity).await;
							upgrade.on_upgrade(move |socket| {
								socket_loop(live, connection, socket, frames)
							})
						}
						Err(error) => failure(&error),
					}
				}
			}),
		);
		for registration in registrations {
			let server = self.clone();
			router = router.route(
				&format!("/api/{}", registration.name),
				post(move |request: Request| {
					let server = server.clone();
					async move { server.serve(registration, request).await }
				}),
			);
		}
		router
	}

	async fn serve(&self, registration: &'static Registration, request: Request) -> Response {
		let (parts, body) = request.into_parts();
		let Ok(bytes) = axum::body::to_bytes(body, BODY_LIMIT).await else {
			return failure(&Error::invalid(
				"the request body is too large, or didn't arrive",
			));
		};
		let arguments = if bytes.iter().all(u8::is_ascii_whitespace) {
			Value::Object(serde_json::Map::new())
		} else {
			match serde_json::from_slice::<Value>(&bytes) {
				Ok(value @ Value::Object(_)) => value,
				Ok(_) | Err(_) => {
					return failure(&Error::invalid(
						"send the arguments as one JSON object, such as `{\"limit\": 10}`",
					));
				}
			}
		};
		let request_id = (registration.kind == Kind::Mutation)
			.then(|| {
				parts
					.headers
					.get(IDEMPOTENCY_KEY)
					.map(|value| value.to_str().map(str::to_owned))
			})
			.flatten()
			.transpose();
		let Ok(request_id) = request_id else {
			return failure(&Error::invalid(
				"the idempotency key must be printable ASCII",
			));
		};
		match self.call(&parts, registration, arguments, request_id).await {
			Ok(value) => {
				(
					[(
						header::CONTENT_TYPE,
						HeaderValue::from_static("application/json"),
					)],
					value.to_string(),
				)
					.into_response()
			}
			Err(error) => failure(&error),
		}
	}

	/// Runs `registration` for the request `parts` came with.
	pub(crate) async fn call(
		&self,
		parts: &http::request::Parts,
		registration: &'static Registration,
		arguments: Value,
		request_id: Option<String>,
	) -> crate::Result<Value> {
		let identity = self.inner.identify.identify(parts).await?;
		let db = self.inner.db.with_context(identity.context.clone());
		(registration.handler)(Invocation {
			db,
			identity,
			request_id,
			arguments,
		})
		.await
	}
}

impl std::fmt::Debug for Server {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("Server").finish_non_exhaustive()
	}
}

static INSTALLED: RwLock<Option<Server>> = RwLock::new(None);

/// The server in-process calls use, once one is installed.
pub(crate) fn installed() -> Option<Server> {
	INSTALLED
		.read()
		.unwrap_or_else(PoisonError::into_inner)
		.clone()
}

/// The answer to a failed call: the status, and `Kind|message`.
fn failure(error: &Error) -> Response {
	let status = StatusCode::from_u16(error.status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
	let mut response = Response::new(Body::from(error.to_wire()));
	*response.status_mut() = status;
	response
		.headers_mut()
		.insert(header::CONTENT_TYPE, HeaderValue::from_static("text/plain"));
	response
}

/// One WebSocket connection: frames out from the hub, frames in from the
/// client, until either side closes.
async fn socket_loop(
	hub: Arc<LiveHub>,
	connection: LiveConnection,
	socket: WebSocket,
	mut frames: tokio::sync::mpsc::UnboundedReceiver<ServerFrame>,
) {
	let (mut sender, mut receiver) = socket.split();

	let outgoing = async {
		while let Some(frame) = frames.recv().await {
			let Ok(text) = serde_json::to_string(&frame) else {
				continue;
			};

			if sender.send(Message::Text(text.into())).await.is_err() {
				break;
			}
		}
	};

	let incoming = async {
		while let Some(Ok(message)) = receiver.next().await {
			match message {
				Message::Text(text) => {
					let Ok(frame) = serde_json::from_str::<ClientFrame>(text.as_str()) else {
						continue;
					};

					match frame {
						ClientFrame::Sub { query, name, args } => {
							if let Err(error) = hub.subscribe(&connection, query, &name, args).await
							{
								let _ = connection.send(ServerFrame::Transition {
									from: 0,
									to: hub.version(),
									updates: vec![Update {
										query,
										outcome: UpdateOutcome::Error {
											error: error.into(),
										},
									}],
								});
							}
						}
						ClientFrame::Unsub { query } => {
							hub.unsubscribe(&connection, query).await;
						}
						ClientFrame::Mutate {
							request,
							name,
							args,
						} => {
							let _ = hub.mutate(&connection, &request, &name, args).await;
						}
						ClientFrame::Ping => {
							let _ = connection.send(ServerFrame::Pong);
						}
						ClientFrame::Hello { .. } | ClientFrame::Auth { .. } => {}
					}
				}
				Message::Close(_) => break,
				Message::Ping(_) | Message::Pong(_) | Message::Binary(_) => {}
			}
		}
	};

	tokio::select! {
		() = outgoing => {}
		() = incoming => {}
	}
}
