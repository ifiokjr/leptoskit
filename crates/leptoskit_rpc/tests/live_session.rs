//! The live session's own limits, leases, authentication changes, and
//! frame splitting, driven over a WebSocket: what one connection may hold,
//! what a resume may ask for, when its identity is checked again, and how a
//! transition too large for one frame is split.

#![cfg(all(feature = "live", feature = "sqlite"))]
#![allow(
	clippy::cognitive_complexity,
	reason = "each scenario reads as one story, step by step"
)]

use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::time::Duration;
use std::time::SystemTime;

use futures::SinkExt;
use futures::Stream;
use futures::StreamExt;
use leptoskit_db::Db;
use leptoskit_db::Id;
use leptoskit_db::table;
use leptoskit_rpc as rpc;
use leptoskit_rpc::live::ClientFrame;
use leptoskit_rpc::live::Resume;
use leptoskit_rpc::live::ResumeQuery;
use leptoskit_rpc::live::ServerFrame;
use leptoskit_rpc::live::Update;
use leptoskit_rpc::server::Identity;
use leptoskit_rpc::server::IdentityLease;
use leptoskit_rpc::server::Live;
use serde_json::Value;
use serde_json::json;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use tokio_tungstenite::MaybeTlsStream;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

/// A note everyone may read.
#[table(name = "session_notes", live)]
pub struct Note {
	pub id: Id<Self>,
	pub body: String,
}

/// Every note's body, in order.
#[rpc::query]
pub async fn session_bodies(cx: &rpc::QueryCx) -> rpc::Result<Vec<String>> {
	let notes = Note::query()
		.order_by(|note| note.body.asc())
		.all(cx.db())
		.await?;
	Ok(notes.into_iter().map(|note| note.body).collect())
}

/// Every note's body, read slowly, so a commit can land while it runs.
#[rpc::query]
pub async fn session_slow_bodies(cx: &rpc::QueryCx) -> rpc::Result<Vec<String>> {
	tokio::time::sleep(Duration::from_millis(300)).await;
	let notes = Note::query()
		.order_by(|note| note.body.asc())
		.all(cx.db())
		.await?;
	Ok(notes.into_iter().map(|note| note.body).collect())
}

/// One large document, for a transition that outgrows a frame.
#[table(name = "session_docs", live)]
pub struct Doc {
	pub id: Id<Self>,
	pub body: String,
}

/// The document's body.
#[rpc::query]
pub async fn session_doc(cx: &rpc::QueryCx) -> rpc::Result<String> {
	let doc = Doc::query().first(cx.db()).await?;
	Ok(doc.map(|doc| doc.body).unwrap_or_default())
}

/// Who the connection reads as.
#[rpc::query]
pub async fn session_who(cx: &rpc::QueryCx) -> rpc::Result<String> {
	Ok(cx
		.identity()
		.caller
		.clone()
		.unwrap_or_else(|| "nobody".to_owned()))
}

/// A caller's private notes: only `ada` may read them.
#[rpc::query]
pub async fn session_private(cx: &rpc::QueryCx) -> rpc::Result<Vec<String>> {
	if cx.identity().caller.as_deref() != Some("ada") {
		return Err(rpc::Error::forbidden("not yours"));
	}

	Ok(vec!["ada's".to_owned()])
}

/// A note everyone may write.
#[rpc::mutation]
pub async fn session_add(cx: &rpc::MutationCx, body: String) -> rpc::Result<()> {
	NewNote { body }.insert(cx.tx()).await?;
	Ok(())
}

/// Waits until the test lets it finish.
static SLOW: tokio::sync::Notify = tokio::sync::Notify::const_new();

/// An action that finishes only when the test says.
#[rpc::action]
pub async fn session_slow(_cx: &rpc::ActionCx) -> rpc::Result<()> {
	SLOW.notified().await;
	Ok(())
}

/// Where a burst stream reached.
#[derive(Clone, serde::Deserialize, serde::Serialize)]
pub struct Step {
	pub n: u32,
}

impl rpc::Resumable for Step {
	type Cursor = u32;

	fn cursor(&self) -> u32 {
		self.n
	}
}

/// A hundred items at once, then nothing until cancelled.
#[rpc::stream(resumable)]
pub async fn session_burst(
	_cx: &rpc::QueryCx,
	cursor: Option<u32>,
) -> rpc::Result<impl Stream<Item = Step>> {
	let start = cursor.map_or(0, |cursor| cursor + 1);
	Ok(futures::stream::iter((start..100).map(|n| Step { n })).chain(futures::stream::pending()))
}

/// What a member shows the others.
#[derive(Clone, Debug, Default, serde::Deserialize, PartialEq, serde::Serialize)]
pub struct Seat {
	pub name: String,
}

/// A room, open to anyone but `eve`.
#[rpc::channel(message = String, presence = Seat, authorize = may_join)]
pub struct Room {
	pub room: String,
}

#[allow(clippy::unused_async, reason = "the signature the macro expects")]
async fn may_join(cx: &rpc::QueryCx, room: &Room) -> rpc::Result<()> {
	if cx.identity().caller.as_deref() == Some("eve") && room.room != "public" {
		return Err(rpc::Error::forbidden("eve may not join"));
	}

	Ok(())
}

struct Server {
	url: String,
	task: JoinHandle<()>,
}

impl Drop for Server {
	fn drop(&mut self) {
		self.task.abort();
	}
}

async fn serve(db: Db, identify: impl rpc::server::Identify) -> Server {
	let live = Live::new(db).identify(identify);
	let app = live.router::<()>();
	let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
	let address = listener.local_addr().unwrap();
	let task = tokio::spawn(async move {
		axum::serve(listener, app).await.unwrap();
	});
	Server {
		url: format!("ws://{address}/live"),
		task,
	}
}

type Socket = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

/// Tells callers apart by an `x-caller` header or a bearer token.
struct Caller;

impl rpc::server::Identify for Caller {
	fn identify<'a>(
		&'a self,
		request: &'a http::request::Parts,
	) -> Pin<Box<dyn Future<Output = rpc::Result<Identity>> + Send + 'a>> {
		Box::pin(async move {
			let header = |name: &str| {
				request
					.headers
					.get(name)
					.and_then(|value| value.to_str().ok())
					.map(str::to_owned)
			};
			let caller = header("x-caller").or_else(|| {
				header("authorization")
					.and_then(|value| value.strip_prefix("Bearer ").map(str::to_owned))
			});

			Ok(caller.map_or_else(Identity::anonymous, |caller| {
				Identity::anonymous().with_caller(caller)
			}))
		})
	}
}

/// The lease a connection gets, for the deadlines the session watches.
#[derive(Clone, Copy)]
enum Lease {
	Refreshing,
	Expiring,
	Expired,
	Refused,
	Broken,
	Slow,
}

impl rpc::server::Identify for Lease {
	fn identify<'a>(
		&'a self,
		_request: &'a http::request::Parts,
	) -> Pin<Box<dyn Future<Output = rpc::Result<Identity>> + Send + 'a>> {
		Box::pin(std::future::ready(Ok(
			Identity::anonymous().with_caller("ada")
		)))
	}

	fn lease<'a>(
		&'a self,
		_identity: &'a Identity,
	) -> Pin<Box<dyn Future<Output = rpc::Result<IdentityLease>> + Send + 'a>> {
		let lease = match self {
			Self::Refreshing => {
				Ok(IdentityLease {
					refresh_at: Some(SystemTime::now() + Duration::from_millis(80)),
					..IdentityLease::default()
				})
			}
			Self::Expiring => {
				Ok(IdentityLease {
					expires_at: Some(SystemTime::now() + Duration::from_millis(250)),
					..IdentityLease::default()
				})
			}
			Self::Expired => {
				Ok(IdentityLease {
					expires_at: Some(SystemTime::now() - Duration::from_secs(1)),
					..IdentityLease::default()
				})
			}
			Self::Refused => Err(rpc::Error::unauthorized("signed out")),
			Self::Broken => Err(rpc::Error::retry("the database is down")),
			Self::Slow => {
				return Box::pin(async {
					tokio::time::sleep(Duration::from_millis(80)).await;
					Ok(IdentityLease::default())
				});
			}
		};
		Box::pin(std::future::ready(lease))
	}
}

/// A lease that can start refusing after the connection is open.
struct Switch {
	deny: Arc<AtomicBool>,
}

impl rpc::server::Identify for Switch {
	fn identify<'a>(
		&'a self,
		request: &'a http::request::Parts,
	) -> Pin<Box<dyn Future<Output = rpc::Result<Identity>> + Send + 'a>> {
		Box::pin(async move {
			let caller = request
				.headers
				.get("x-caller")
				.and_then(|value| value.to_str().ok())
				.map(str::to_owned);
			Ok(caller.map_or_else(Identity::anonymous, |caller| {
				Identity::anonymous().with_caller(caller)
			}))
		})
	}

	fn identify_token<'a>(
		&'a self,
		token: &'a str,
	) -> Pin<Box<dyn Future<Output = rpc::Result<Identity>> + Send + 'a>> {
		Box::pin(async move {
			Ok(if token == "anon" {
				Identity::anonymous()
			} else {
				Identity::anonymous().with_caller(token)
			})
		})
	}

	fn lease<'a>(
		&'a self,
		_identity: &'a Identity,
	) -> Pin<Box<dyn Future<Output = rpc::Result<IdentityLease>> + Send + 'a>> {
		let deny = self.deny.load(Ordering::SeqCst);
		Box::pin(std::future::ready(if deny {
			Err(rpc::Error::unauthorized("signed out"))
		} else {
			Ok(IdentityLease::default())
		}))
	}
}

async fn connect(server: &Server, caller: Option<&str>) -> Socket {
	let mut request = server.url.as_str().into_client_request().unwrap();

	if let Some(caller) = caller {
		request
			.headers_mut()
			.insert("x-caller", caller.parse().unwrap());
	}

	let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
	let ServerFrame::Welcome { .. } = next(&mut socket).await else {
		panic!("welcome first");
	};
	socket
}

async fn send(socket: &mut Socket, frame: ClientFrame) {
	let text = serde_json::to_string(&frame).unwrap();
	socket.send(Message::Text(text.into())).await.unwrap();
}

async fn next(socket: &mut Socket) -> ServerFrame {
	loop {
		let message = tokio::time::timeout(Duration::from_secs(20), socket.next())
			.await
			.expect("a frame arrives")
			.unwrap()
			.unwrap();

		match message {
			Message::Text(text) => return serde_json::from_str(&text).unwrap(),
			Message::Ping(_) | Message::Pong(_) => {}
			other => panic!("unexpected frame: {other:?}"),
		}
	}
}

async fn next_within(socket: &mut Socket, wait: Duration) -> Option<ServerFrame> {
	tokio::time::timeout(wait, next(socket)).await.ok()
}

fn subscribe(id: u32, query: &str) -> ClientFrame {
	ClientFrame::Sub {
		id,
		query: query.into(),
		args: json!({}),
	}
}

/// The first value of a subscription, or why it was refused.
async fn answer(socket: &mut Socket, id: u32) -> Result<Value, rpc::live::WireError> {
	match next(socket).await {
		ServerFrame::Done {
			request, ok, err, ..
		} if request == id => err.map_or_else(|| Ok(ok.unwrap_or(Value::Null)), Err),
		other => panic!("expected the answer to {id}, got {other:?}"),
	}
}

async fn still_open(socket: &mut Socket) {
	send(socket, ClientFrame::Ping).await;
	assert_eq!(next(socket).await, ServerFrame::Pong);
}

fn room_key(room: &str) -> String {
	json!({ "room": room }).to_string()
}

async fn join(socket: &mut Socket, room: &str, name: &str) {
	send(
		socket,
		ClientFrame::Join {
			channel: room_key(room),
			name: "room".into(),
			args: json!({ "room": room }),
			state: json!({ "name": name }),
		},
	)
	.await;
}

async fn presence(socket: &mut Socket) -> Vec<String> {
	let ServerFrame::Presence { members, .. } = next(socket).await else {
		panic!("presence");
	};
	members.into_iter().map(|member| member.id).collect()
}

/// A connection may hold only so many subscriptions, and unsubscribing
/// frees one.
#[tokio::test(flavor = "multi_thread")]
async fn subscriptions_stop_at_the_limit_and_an_id_can_be_freed() {
	let server = serve(leptoskit_db::testing::sqlite().await, Caller).await;
	let mut socket = connect(&server, None).await;

	for id in 1..=u32::try_from(rpc::live::MAX_SUBSCRIPTIONS).unwrap() {
		send(&mut socket, subscribe(id, "session_bodies")).await;
		assert_eq!(answer(&mut socket, id).await.unwrap(), json!([]));
	}

	send(&mut socket, subscribe(1_000, "session_bodies")).await;
	let error = answer(&mut socket, 1_000).await.unwrap_err();
	assert!(
		error.message.contains("subscribe to 256 queries"),
		"{}",
		error.message
	);

	send(&mut socket, ClientFrame::Unsub { id: 1 }).await;
	send(&mut socket, subscribe(1_000, "session_bodies")).await;
	assert_eq!(answer(&mut socket, 1_000).await.unwrap(), json!([]));
	still_open(&mut socket).await;
}

/// A resume with nothing to do changes nothing, and one over the limit
/// answers each query it can, refusing the rest.
#[tokio::test(flavor = "multi_thread")]
async fn resuming_with_nothing_does_nothing_and_over_the_limit_is_refused() {
	let db = leptoskit_db::testing::sqlite().await;
	NewNote {
		body: "before".into(),
	}
	.insert(&db)
	.await
	.unwrap();
	let server = serve(db, Caller).await;
	let mut socket = connect(&server, None).await;

	send(
		&mut socket,
		ClientFrame::Hello {
			resume: Some(Resume {
				version: 0,
				queries: Vec::new(),
			}),
		},
	)
	.await;
	still_open(&mut socket).await;

	let queries: Vec<ResumeQuery> = (1..=u32::try_from(rpc::live::MAX_SUBSCRIPTIONS + 1).unwrap())
		.map(|id| {
			ResumeQuery {
				id,
				query: "session_bodies".into(),
				args: json!({}),
			}
		})
		.collect();
	send(
		&mut socket,
		ClientFrame::Hello {
			resume: Some(Resume {
				version: 0,
				queries,
			}),
		},
	)
	.await;

	let ServerFrame::Transition(transition) = next(&mut socket).await else {
		panic!("the resume answers in one transition");
	};
	assert_eq!(transition.updates.len(), rpc::live::MAX_SUBSCRIPTIONS + 1);
	assert_eq!(
		transition
			.updates
			.iter()
			.filter(|update| matches!(update.update, Update::Ok(_)))
			.count(),
		rpc::live::MAX_SUBSCRIPTIONS
	);
	let refused = transition
		.updates
		.iter()
		.filter_map(|update| {
			match &update.update {
				Update::Err(error) => Some(error.message.as_str()),
				_ => None,
			}
		})
		.collect::<Vec<_>>();
	assert_eq!(refused.len(), 1);
	assert!(
		refused[0].contains("subscribe to 256 queries"),
		"{refused:?}"
	);
}

/// A resume naming a query the server doesn't know answers it as an error.
#[tokio::test(flavor = "multi_thread")]
async fn resuming_an_unknown_query_answers_it_as_an_error() {
	let server = serve(leptoskit_db::testing::sqlite().await, Caller).await;
	let mut socket = connect(&server, None).await;

	send(
		&mut socket,
		ClientFrame::Hello {
			resume: Some(Resume {
				version: 0,
				queries: vec![ResumeQuery {
					id: 3,
					query: "no_such_query".into(),
					args: json!({}),
				}],
			}),
		},
	)
	.await;

	let ServerFrame::Transition(transition) = next(&mut socket).await else {
		panic!("the resume answers in one transition");
	};
	assert_eq!(transition.updates.len(), 1);
	match &transition.updates[0].update {
		Update::Err(error) => {
			assert!(
				error.message.contains("no operation is named"),
				"{}",
				error.message
			);
		}
		other => panic!("expected an error, got {other:?}"),
	}
}

/// A channel's key and its presence each have a byte limit.
#[tokio::test(flavor = "multi_thread")]
async fn channel_keys_and_presence_have_limits() {
	let server = serve(leptoskit_db::testing::sqlite().await, Caller).await;
	let mut socket = connect(&server, None).await;

	let room = "r".repeat(rpc::live::MAX_CHANNEL_KEY_BYTES);
	let args = json!({ "room": room });
	send(
		&mut socket,
		ClientFrame::Join {
			channel: serde_json::to_string(&args).unwrap(),
			name: "room".into(),
			args,
			state: json!(null),
		},
	)
	.await;
	let ServerFrame::ChannelError { err, .. } = next(&mut socket).await else {
		panic!("a refusal");
	};
	assert!(
		err.message.contains("at most 4096 bytes"),
		"{}",
		err.message
	);

	send(
		&mut socket,
		ClientFrame::Join {
			channel: room_key("lobby"),
			name: "room".into(),
			args: json!({ "room": "lobby" }),
			state: json!({ "name": "x".repeat(rpc::live::MAX_PRESENCE_BYTES) }),
		},
	)
	.await;
	let ServerFrame::ChannelError { err, .. } = next(&mut socket).await else {
		panic!("a refusal");
	};
	assert!(
		err.message.contains("at most 16384 bytes"),
		"{}",
		err.message
	);

	join(&mut socket, "lobby", "ada").await;
	assert_eq!(presence(&mut socket).await, ["live-1"]);
	// Joining the same instance again replaces the member rather than
	// adding one.
	join(&mut socket, "lobby", "ada").await;
	assert_eq!(presence(&mut socket).await, ["live-1"]);
	send(
		&mut socket,
		ClientFrame::Leave {
			channel: room_key("lobby"),
		},
	)
	.await;
	// A signal to a room the connection has left changes nothing.
	send(
		&mut socket,
		ClientFrame::Signal {
			channel: room_key("lobby"),
			message: json!("hello"),
		},
	)
	.await;
	still_open(&mut socket).await;
}

/// An identity's lease refreshes it, expires it, or asks the client back.
#[tokio::test(flavor = "multi_thread")]
async fn lease_refresh_expiry_and_failures() {
	let refreshing = serve(leptoskit_db::testing::sqlite().await, Lease::Refreshing).await;
	let mut socket = connect(&refreshing, None).await;
	let frame = next_within(&mut socket, Duration::from_secs(2))
		.await
		.expect("the refresh arrives");
	assert_eq!(
		frame,
		ServerFrame::AuthStatus {
			refused: false,
			expiring: true,
		}
	);
	still_open(&mut socket).await;

	let expiring = serve(leptoskit_db::testing::sqlite().await, Lease::Expiring).await;
	let mut socket = connect(&expiring, None).await;
	let frame = next_within(&mut socket, Duration::from_secs(2))
		.await
		.expect("the refusal arrives");
	assert_eq!(
		frame,
		ServerFrame::AuthStatus {
			refused: true,
			expiring: false,
		}
	);
	assert!(
		tokio::time::timeout(Duration::from_secs(2), async {
			while socket.next().await.is_some() {}
		})
		.await
		.is_ok(),
		"the connection closes"
	);

	let refused = serve(leptoskit_db::testing::sqlite().await, Lease::Refused).await;
	let mut socket = connect(&refused, None).await;
	let frame = next_within(&mut socket, Duration::from_secs(2))
		.await
		.expect("the refusal arrives");
	assert_eq!(
		frame,
		ServerFrame::AuthStatus {
			refused: true,
			expiring: false,
		}
	);

	let expired = serve(leptoskit_db::testing::sqlite().await, Lease::Expired).await;
	let mut socket = connect(&expired, None).await;
	let frame = next_within(&mut socket, Duration::from_secs(2))
		.await
		.expect("the refusal arrives");
	assert_eq!(
		frame,
		ServerFrame::AuthStatus {
			refused: true,
			expiring: false,
		}
	);

	let broken = serve(leptoskit_db::testing::sqlite().await, Lease::Broken).await;
	let mut socket = connect(&broken, None).await;
	let frame = next_within(&mut socket, Duration::from_secs(2))
		.await
		.expect("the reconnect arrives");
	assert_eq!(frame, ServerFrame::Reconnect { after_ms: 500 });
}

/// A log that keeps moving while the lease is read makes the connection
/// reconnect rather than register a stale identity.
#[tokio::test(flavor = "multi_thread")]
async fn a_busy_log_asks_the_connection_back() {
	let db = leptoskit_db::testing::sqlite().await;
	let server = serve(db.clone(), Lease::Slow).await;
	let writer = tokio::spawn(async move {
		for n in 0..100 {
			NewNote {
				body: format!("note {n}"),
			}
			.insert(&db)
			.await
			.unwrap();
			tokio::time::sleep(Duration::from_millis(10)).await;
		}
	});

	let mut socket = connect(&server, None).await;
	let frame = next_within(&mut socket, Duration::from_secs(5))
		.await
		.expect("the reconnect arrives");
	assert_eq!(frame, ServerFrame::Reconnect { after_ms: 250 });
	writer.abort();
}

/// Authenticating reads every subscription and channel again, and a
/// refusal reaches both.
#[tokio::test(flavor = "multi_thread")]
async fn authentication_rereads_subscriptions_and_channels() {
	let server = serve(leptoskit_db::testing::sqlite().await, Caller).await;
	let mut socket = connect(&server, None).await;

	send(&mut socket, subscribe(1, "session_who")).await;
	assert_eq!(answer(&mut socket, 1).await.unwrap(), json!("nobody"));
	join(&mut socket, "lobby", "anonymous").await;
	assert_eq!(presence(&mut socket).await, ["live-1"]);

	send(
		&mut socket,
		ClientFrame::Auth {
			token: "ada".into(),
		},
	)
	.await;
	let mut seen_who = None;
	let mut seen_presence = None;
	loop {
		match next(&mut socket).await {
			ServerFrame::Transition(transition) => {
				if let Some(update) = transition.updates.iter().find(|update| update.id == 1)
					&& let Update::Ok(who) = &update.update
				{
					seen_who = Some(who.clone());
				}
			}
			ServerFrame::Presence { members, .. } => {
				seen_presence = Some(
					members
						.into_iter()
						.map(|member| member.id)
						.collect::<Vec<_>>(),
				);
			}
			ServerFrame::AuthStatus { refused, .. } => {
				assert!(!refused);
				break;
			}
			other => panic!("unexpected frame: {other:?}"),
		}
	}
	assert_eq!(seen_who, Some(json!("ada")));
	assert_eq!(seen_presence, Some(vec!["ada".to_owned()]));

	// `eve` may not read the private query or join the room: both say so.
	join(&mut socket, "public", "ada").await;
	assert_eq!(presence(&mut socket).await, ["ada"]);
	send(&mut socket, subscribe(2, "session_private")).await;
	assert_eq!(answer(&mut socket, 2).await.unwrap(), json!(["ada's"]));

	send(
		&mut socket,
		ClientFrame::Auth {
			token: "eve".into(),
		},
	)
	.await;
	let mut seen_refusal = false;
	let mut seen_channel_error = false;
	loop {
		match next(&mut socket).await {
			ServerFrame::Transition(transition) => {
				assert!(
					transition
						.updates
						.iter()
						.any(|update| update.id == 1 && matches!(update.update, Update::Ok(_)))
				);
				seen_refusal |= transition
					.updates
					.iter()
					.any(|update| matches!(update.update, Update::Err(_)));
			}
			ServerFrame::ChannelError { err, .. } => {
				assert!(err.message.contains("eve may not join"), "{}", err.message);
				seen_channel_error = true;
			}
			ServerFrame::Presence { .. } => {}
			ServerFrame::AuthStatus { refused, .. } => {
				assert!(!refused);
				break;
			}
			other => panic!("unexpected frame: {other:?}"),
		}
	}
	assert!(seen_refusal, "the private query was read as eve");
	assert!(seen_channel_error, "the room was checked as eve");

	// Back to nobody: the public room keeps its member, named by the
	// connection when the identity has no caller.
	join(&mut socket, "public", "someone").await;
	let _ = presence(&mut socket).await;
	send(
		&mut socket,
		ClientFrame::Auth {
			token: String::new(),
		},
	)
	.await;
	loop {
		if matches!(next(&mut socket).await, ServerFrame::AuthStatus { .. }) {
			break;
		}
	}
}

/// An action that finishes after the connection changed identity is
/// refused: its caller is no longer who it ran as.
#[tokio::test(flavor = "multi_thread")]
async fn an_action_that_finishes_after_authentication_is_refused() {
	let server = serve(leptoskit_db::testing::sqlite().await, Caller).await;
	let mut socket = connect(&server, None).await;

	send(
		&mut socket,
		ClientFrame::Act {
			request: 9,
			name: "session_slow".into(),
			args: json!({}),
		},
	)
	.await;
	send(
		&mut socket,
		ClientFrame::Auth {
			token: "ada".into(),
		},
	)
	.await;

	loop {
		match next(&mut socket).await {
			ServerFrame::AuthStatus { .. } => break,
			ServerFrame::Done { .. } => panic!("the action finished before the auth changed"),
			_ => {}
		}
	}

	SLOW.notify_one();
	let ServerFrame::Done {
		request,
		err: Some(error),
		..
	} = next(&mut socket).await
	else {
		panic!("the action is refused");
	};
	assert_eq!(request, 9);
	assert_eq!(error.status, 401);
	assert!(
		error.message.contains("identity changed"),
		"{}",
		error.message
	);
}

/// A transition larger than one frame is split under the limit, all at the
/// same version.
#[tokio::test(flavor = "multi_thread")]
async fn a_large_transition_is_split_under_the_frame_limit() {
	let db = leptoskit_db::testing::sqlite().await;
	let doc = NewDoc {
		body: "a".repeat(400_000),
	}
	.insert(&db)
	.await
	.unwrap();
	let doc_id = doc.id;
	let server = serve(db.clone(), Caller).await;
	let mut socket = connect(&server, None).await;

	for id in 1..=3 {
		send(&mut socket, subscribe(id, "session_doc")).await;
		assert_eq!(
			answer(&mut socket, id)
				.await
				.unwrap()
				.as_str()
				.unwrap()
				.len(),
			400_000
		);
	}

	Doc::update()
		.filter(move |doc| doc.id.eq(doc_id))
		.set(|doc| doc.body, "b".repeat(400_100))
		.run(&db)
		.await
		.unwrap();

	let mut transitions = Vec::new();
	while transitions.len() < 2 {
		if let ServerFrame::Transition(transition) = next(&mut socket).await {
			transitions.push(transition);
		}
	}

	let mut ids = Vec::new();
	let to = transitions[0].to;
	for transition in &transitions {
		assert_eq!(transition.to, to, "every chunk lands on the same version");
		ids.extend(transition.updates.iter().map(|update| update.id));
	}
	ids.sort_unstable();
	assert_eq!(ids, [1, 2, 3]);
	assert!(
		transitions
			.iter()
			.all(|transition| transition.updates.len() < 3),
		"the transition was not split"
	);
}

/// A client that falls far behind gets the whole state in one full
/// transition once it reads again.
#[tokio::test(flavor = "multi_thread")]
async fn a_slow_reader_gets_a_full_transition() {
	let db = leptoskit_db::testing::sqlite().await;
	let doc = NewDoc {
		body: "a".repeat(200_000),
	}
	.insert(&db)
	.await
	.unwrap();
	let doc_id = doc.id;
	let server = serve(db.clone(), Caller).await;
	let mut socket = connect(&server, None).await;
	send(&mut socket, subscribe(1, "session_doc")).await;
	assert_eq!(
		answer(&mut socket, 1)
			.await
			.unwrap()
			.as_str()
			.unwrap()
			.len(),
		200_000
	);

	// Neither the socket nor the session is read while these commit. One
	// at a time, so each makes its own transition.
	for n in 0..40 {
		Doc::update()
			.filter(move |doc| doc.id.eq(doc_id))
			.set(|doc| doc.body, format!("{}{n}", "b".repeat(200_000)))
			.run(&db)
			.await
			.unwrap();
		tokio::time::sleep(Duration::from_millis(30)).await;
	}
	tokio::time::sleep(Duration::from_millis(300)).await;

	// Drain what queued, then change the value once more: the replay says
	// it is full and repeats the subscription.
	let mut full = None;
	for _ in 0..120 {
		match next_within(&mut socket, Duration::from_millis(200)).await {
			Some(ServerFrame::Transition(transition)) => {
				if transition.full && transition.updates.iter().any(|update| update.id == 1) {
					full = Some(transition);
				}
			}
			Some(_) => {}
			None => break,
		}
	}
	if full.is_none() {
		Doc::update()
			.filter(move |doc| doc.id.eq(doc_id))
			.set(|doc| doc.body, "c".repeat(200_001))
			.run(&db)
			.await
			.unwrap();

		for _ in 0..10 {
			match next_within(&mut socket, Duration::from_secs(2)).await {
				Some(ServerFrame::Transition(transition)) => {
					if transition.full && transition.updates.iter().any(|update| update.id == 1) {
						full = Some(transition);
						break;
					}
				}
				Some(_) => {}
				None => break,
			}
		}
	}

	let full = full.expect("a full transition after the slow read");
	assert_eq!(full.updates.len(), 1);
	assert!(matches!(full.updates[0].update, Update::Ok(_)));
}

/// A resume whose query runs while a commit lands catches the commit too.
#[tokio::test(flavor = "multi_thread")]
async fn a_resume_catches_a_commit_that_lands_while_it_runs() {
	let db = leptoskit_db::testing::sqlite().await;
	NewNote {
		body: "before".into(),
	}
	.insert(&db)
	.await
	.unwrap();
	let server = serve(db.clone(), Caller).await;
	let mut socket = connect(&server, None).await;

	send(
		&mut socket,
		ClientFrame::Hello {
			resume: Some(Resume {
				version: 1,
				queries: vec![ResumeQuery {
					id: 1,
					query: "session_slow_bodies".into(),
					args: json!({}),
				}],
			}),
		},
	)
	.await;

	// The query sleeps, so this commit lands after the resume began, and
	// the log is pruned before the resume looks back for it.
	tokio::time::sleep(Duration::from_millis(50)).await;
	NewNote {
		body: "during".into(),
	}
	.insert(&db)
	.await
	.unwrap();
	tokio::time::sleep(Duration::from_millis(50)).await;
	db.system("test")
		.prune_changes(Duration::ZERO)
		.await
		.unwrap();

	let ServerFrame::Transition(transition) = next(&mut socket).await else {
		panic!("the resume's transition");
	};
	assert_eq!(transition.from, 1);
	let value = transition
		.updates
		.iter()
		.find(|update| update.id == 1)
		.and_then(|update| {
			match &update.update {
				Update::Ok(value) => Some(value.clone()),
				_ => None,
			}
		})
		.expect("the resumed value");
	assert_eq!(value, json!(["before", "during"]));
}

/// Only a query can be subscribed to.
#[tokio::test(flavor = "multi_thread")]
async fn subscribing_to_a_mutation_is_refused() {
	let server = serve(leptoskit_db::testing::sqlite().await, Caller).await;
	let mut socket = connect(&server, None).await;

	send(&mut socket, subscribe(4, "session_add")).await;
	let error = answer(&mut socket, 4).await.unwrap_err();
	assert!(
		error.message.contains("only a query can be subscribed to"),
		"{}",
		error.message
	);
	still_open(&mut socket).await;
}

/// Authenticating stops a stream that is still producing, and tells the
/// client it was interrupted.
#[tokio::test(flavor = "multi_thread")]
async fn authentication_interrupts_a_busy_stream() {
	let server = serve(leptoskit_db::testing::sqlite().await, Caller).await;
	let mut socket = connect(&server, None).await;

	send(
		&mut socket,
		ClientFrame::Stream {
			stream: 1,
			name: "session_burst".into(),
			args: json!({}),
			credits: 1_000,
		},
	)
	.await;
	send(&mut socket, subscribe(2, "session_slow_bodies")).await;
	send(
		&mut socket,
		ClientFrame::Auth {
			token: "ada".into(),
		},
	)
	.await;

	let mut interrupted = false;

	loop {
		match next(&mut socket).await {
			ServerFrame::Item { .. }
			| ServerFrame::Done { .. }
			| ServerFrame::Transition(_)
			| ServerFrame::Presence { .. } => {}
			ServerFrame::End { stream, reason, .. } => {
				assert_eq!((stream, reason.as_str()), (1, "interrupted"));
				interrupted = true;
			}
			ServerFrame::AuthStatus { refused, .. } => {
				assert!(!refused);
				break;
			}
			other => panic!("unexpected frame: {other:?}"),
		}
	}

	assert!(interrupted, "the stream was told it was interrupted");
}

/// A refused lease after a bearer sign-in closes the connection, and an
/// identity with no caller is named by its connection in a room.
#[tokio::test(flavor = "multi_thread")]
async fn a_lease_refused_after_signing_in_closes_the_connection() {
	let deny = Arc::new(AtomicBool::new(false));
	let server = serve(
		leptoskit_db::testing::sqlite().await,
		Switch {
			deny: Arc::clone(&deny),
		},
	)
	.await;
	let mut socket = connect(&server, None).await;
	join(&mut socket, "lobby", "anonymous").await;
	assert_eq!(presence(&mut socket).await, ["live-1"]);

	send(
		&mut socket,
		ClientFrame::Auth {
			token: "anon".into(),
		},
	)
	.await;
	let mut member = None;
	loop {
		match next(&mut socket).await {
			ServerFrame::Presence { members, .. } => {
				member = Some(
					members
						.into_iter()
						.map(|member| member.id)
						.collect::<Vec<_>>(),
				);
			}
			ServerFrame::AuthStatus { refused, .. } => {
				assert!(!refused);
				break;
			}
			other => panic!("unexpected frame: {other:?}"),
		}
	}
	assert_eq!(member, Some(vec!["live-1".to_owned()]));

	deny.store(true, Ordering::SeqCst);
	send(
		&mut socket,
		ClientFrame::Auth {
			token: "ada".into(),
		},
	)
	.await;
	let frame = next_within(&mut socket, Duration::from_secs(2))
		.await
		.expect("the refusal arrives");
	assert_eq!(
		frame,
		ServerFrame::AuthStatus {
			refused: true,
			expiring: false,
		}
	);
}
