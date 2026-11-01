//! The rest of L6 on SQLite and Postgres: streams with credits, pinned
//! pages, the offline queue, and re-authenticating an open connection.

#![cfg(all(feature = "live", feature = "postgres", feature = "sqlite"))]
#![allow(
	clippy::cognitive_complexity,
	reason = "each scenario reads as one story, step by step"
)]

#[path = "../../leptoskit_db/tests/support/postgres.rs"]
mod postgres;

use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::Once;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use futures::SinkExt;
use futures::Stream;
use futures::StreamExt;
use http::Request;
use http::StatusCode;
use http_body_util::BodyExt;
use leptoskit_db::Db;
use leptoskit_db::Id;
use leptoskit_db::table;
use leptoskit_query::BoxFuture;
use leptoskit_rpc as rpc;
use leptoskit_rpc::FnError;
use leptoskit_rpc::client::ClientConfig;
use leptoskit_rpc::client::Credentials;
use leptoskit_rpc::client::LiveClient;
use leptoskit_rpc::client::MemoryQueue;
use leptoskit_rpc::client::QueueStore;
use leptoskit_rpc::client::QueuedCall;
use leptoskit_rpc::client::QueuedKind;
use leptoskit_rpc::client::StreamEvent;
use leptoskit_rpc::live::ClientFrame;
use leptoskit_rpc::live::EndReason;
use leptoskit_rpc::live::ServerFrame;
use leptoskit_rpc::live::Transition;
use leptoskit_rpc::live::Update;
use leptoskit_rpc::server::Identity;
use leptoskit_rpc::server::Live;
use leptoskit_rpc::testing::LiveClients;
use postgres::PostgresCluster;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
use serde_json::json;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use tokio::task::LocalSet;
use tokio_tungstenite::MaybeTlsStream;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tower::ServiceExt;

// ---- the app -------------------------------------------------------------

/// A post in a channel's history.
#[table(name = "l6_posts", live)]
pub struct Post {
	pub id: Id<Self>,
	pub channel: String,
	pub body: String,
}

/// A post as a page shows it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PostRow {
	pub id: String,
	pub body: String,
}

/// A note, which only its owner reads.
#[table(name = "l6_notes", live)]
pub struct Note {
	pub id: Id<Self>,
	pub owner: Option<String>,
	pub body: String,
}

/// A channel's history, newest first, a page at a time.
#[rpc::query]
pub async fn l6_history(
	cx: &rpc::QueryCx,
	channel: String,
	page: rpc::PageArgs,
) -> rpc::Result<rpc::Page<PostRow>> {
	let query = Post::query().filter(|post| post.channel.eq(channel));
	let page = rpc::paginate(cx.db(), query, &page, |post| post.id.desc(), |post| post.id).await?;
	Ok(rpc::Page {
		items: page
			.items
			.into_iter()
			.map(|post| {
				PostRow {
					id: post.id.to_string(),
					body: post.body,
				}
			})
			.collect(),
		end: page.end,
		has_more: page.has_more,
	})
}

/// The caller's own notes' bodies.
#[rpc::query]
pub async fn l6_mine(cx: &rpc::QueryCx) -> rpc::Result<Vec<String>> {
	let Some(caller) = cx.identity().caller.clone() else {
		return Ok(Vec::new());
	};
	let notes = Note::query()
		.filter(|note| note.owner.eq(caller))
		.order_by(|note| note.id.asc())
		.all(cx.db())
		.await?;
	Ok(notes.into_iter().map(|note| note.body).collect())
}

/// Every note's body, in the order they were written.
#[rpc::query]
pub async fn l6_bodies(cx: &rpc::QueryCx) -> rpc::Result<Vec<String>> {
	let notes = Note::query()
		.order_by(|note| note.id.asc())
		.all(cx.db())
		.await?;
	Ok(notes.into_iter().map(|note| note.body).collect())
}

/// Writes a note as nobody's.
#[rpc::mutation]
pub async fn l6_write(cx: &rpc::MutationCx, body: String) -> rpc::Result<()> {
	NewNote { owner: None, body }.insert(cx.tx()).await?;
	Ok(())
}

/// Writes a note through a mutation, from an action.
#[rpc::action]
pub async fn l6_write_later(cx: &rpc::ActionCx, body: String) -> rpc::Result<()> {
	cx.mutate(L6Write { body }).await
}

/// How far each test run's streams got, by run.
static PRODUCED: Mutex<Option<HashMap<String, Progress>>> = Mutex::new(None);

#[derive(Clone, Debug, Default)]
struct Progress {
	produced: u32,
	dropped: bool,
	opened_at: Vec<Option<u32>>,
}

fn progress(run: &str, change: impl FnOnce(&mut Progress)) -> Progress {
	let mut all = PRODUCED.lock().unwrap();
	let entry = all
		.get_or_insert_with(HashMap::new)
		.entry(run.to_owned())
		.or_default();
	change(entry);
	entry.clone()
}

/// Marks its run's stream dropped when it goes.
struct Guard(String);

impl Drop for Guard {
	fn drop(&mut self) {
		progress(&self.0, |progress| progress.dropped = true);
	}
}

/// The numbers up to `upto`, counting each one it reads.
#[rpc::stream]
pub async fn l6_count(
	cx: &rpc::QueryCx,
	run: String,
	upto: u32,
) -> rpc::Result<impl Stream<Item = u32>> {
	let _ = cx;
	Ok(futures::stream::unfold(
		(0, Guard(run)),
		move |(next, guard)| {
			async move {
				(next < upto).then(|| {
					progress(&guard.0, |progress| progress.produced += 1);
					(next, (next + 1, guard))
				})
			}
		},
	))
}

/// One number, then a failure; or no stream at all for `refuse`.
#[rpc::stream]
pub async fn l6_flaky(
	cx: &rpc::QueryCx,
	refuse: bool,
) -> rpc::Result<impl Stream<Item = rpc::Result<u32>>> {
	let _ = cx;

	if refuse {
		return Err(rpc::Error::forbidden("not this stream"));
	}

	Ok(futures::stream::iter([
		Ok(1),
		Err(rpc::Error::conflict("the stream broke")),
		Ok(2),
	]))
}

/// A tick of a resumable stream.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Tick {
	pub n: u32,
}

impl rpc::Resumable for Tick {
	type Cursor = u32;

	fn cursor(&self) -> u32 {
		self.n
	}
}

/// Ticks forever, from after `cursor`.
#[rpc::stream(resumable)]
pub async fn l6_ticks(
	cx: &rpc::QueryCx,
	run: String,
	cursor: Option<u32>,
) -> rpc::Result<impl Stream<Item = Tick>> {
	let _ = cx;
	progress(&run, |progress| progress.opened_at.push(cursor));
	let start = cursor.map_or(0, |cursor| cursor + 1);
	Ok(futures::stream::unfold(start, |n| {
		async move {
			tokio::time::sleep(Duration::from_millis(5)).await;
			Some((Tick { n }, n + 1))
		}
	}))
}

/// A model's answer, a word at a time.
///
/// # Arguments
///
/// * `prompt` - What to answer.
#[rpc::api(tag = "ai")]
#[rpc::stream]
pub async fn l6_answer(
	cx: &rpc::QueryCx,
	prompt: String,
) -> rpc::Result<impl Stream<Item = rpc::Result<String>>> {
	let _ = cx;
	let words: Vec<rpc::Result<String>> = prompt
		.split_whitespace()
		.map(|word| Ok(word.to_owned()))
		.collect();
	Ok(futures::stream::iter(words))
}

/// A published stream joins the spec as a stream, describing one item, and
/// the `AsyncAPI` document; it has no one-shot route.
#[test]
fn a_published_stream_joins_the_spec() {
	let spec = rpc::spec::collect(&rpc::spec::SpecOptions::new("Live", "1.0.0")).unwrap();
	let answer = spec
		.functions
		.iter()
		.find(|function| function.name == "l6_answer")
		.expect("published");
	assert_eq!(answer.kind, rpc::spec::ApiKind::Stream);
	assert_eq!(answer.output_schema, json!({ "type": "string" }));
	assert_eq!(answer.params[0].name, "prompt");
	assert_eq!(answer.docs, "A model's answer, a word at a time.");
	assert!(spec.to_openapi()["paths"].get("/api/l6_answer").is_none());
	let asyncapi = spec.to_asyncapi();
	assert_eq!(
		asyncapi["operations"]["l6_answer.open"]["action"],
		"receive"
	);
}

/// Tells callers apart by an `x-caller` header, or a bearer token.
struct Header;

impl rpc::server::Identify for Header {
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

			Ok(match caller {
				Some(caller) => Identity::anonymous().with_caller(caller),
				None => Identity::anonymous(),
			})
		})
	}
}

// ---- the server, over a WebSocket ------------------------------------------

struct Server {
	db: Db,
	app: Router,
	url: String,
	task: JoinHandle<()>,
}

impl Drop for Server {
	fn drop(&mut self) {
		self.task.abort();
	}
}

async fn serve(db: Db) -> Server {
	let live = Live::new(db.clone()).identify(Header);
	let app = live.router::<()>();
	let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
	let address = listener.local_addr().unwrap();
	let served = app.clone();
	let task = tokio::spawn(async move {
		axum::serve(listener, served).await.unwrap();
	});
	Server {
		db,
		app,
		url: format!("ws://{address}/live"),
		task,
	}
}

type Socket = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

async fn connect(server: &Server) -> Socket {
	let request = server.url.as_str().into_client_request().unwrap();
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
	next_within(socket, Duration::from_secs(20))
		.await
		.expect("a frame arrives")
}

async fn next_within(socket: &mut Socket, wait: Duration) -> Option<ServerFrame> {
	loop {
		let message = tokio::time::timeout(wait, socket.next())
			.await
			.ok()?
			.unwrap()
			.unwrap();

		match message {
			Message::Text(text) => return Some(serde_json::from_str(&text).unwrap()),
			Message::Ping(_) | Message::Pong(_) => {}
			other => panic!("unexpected frame: {other:?}"),
		}
	}
}

fn open(stream: u32, name: &str, args: Value, credits: u32) -> ClientFrame {
	ClientFrame::Stream {
		stream,
		name: name.into(),
		args,
		credits,
	}
}

fn item(frame: &ServerFrame) -> Option<(u32, &Value)> {
	match frame {
		ServerFrame::Item { stream, value, .. } => Some((*stream, value)),
		_ => None,
	}
}

fn end(frame: &ServerFrame) -> Option<(u32, EndReason)> {
	match frame {
		ServerFrame::End {
			stream,
			reason,
			err,
		} => Some((*stream, EndReason::from_frame(reason, err.clone()))),
		_ => None,
	}
}

/// Every run's own name, so the SQLite and Postgres runs don't share one.
fn run_name(what: &str) -> String {
	format!("{what}-{}", Id::<Post>::new())
}

// ---- streams -------------------------------------------------------------

/// A slow consumer: the server reads only as many items as it was granted,
/// however long the client waits, and more only once it grants more.
async fn streams_wait_for_credits(db: Db) {
	let run = run_name("credits");
	let server = serve(db).await;
	let mut socket = connect(&server).await;
	send(
		&mut socket,
		open(1, "l6_count", json!({ "run": run, "upto": 100 }), 3),
	)
	.await;

	for expected in 0..3 {
		let frame = next(&mut socket).await;
		assert_eq!(item(&frame), Some((1, &json!(expected))), "{frame:?}");
	}

	assert!(
		next_within(&mut socket, Duration::from_millis(300))
			.await
			.is_none(),
		"nothing past the credits"
	);
	assert_eq!(progress(&run, |_| {}).produced, 3, "nothing read ahead");

	send(&mut socket, ClientFrame::Credit { stream: 1, n: 2 }).await;

	for expected in 3..5 {
		let frame = next(&mut socket).await;
		assert_eq!(item(&frame), Some((1, &json!(expected))));
	}

	assert!(
		next_within(&mut socket, Duration::from_millis(200))
			.await
			.is_none()
	);
	assert_eq!(progress(&run, |_| {}).produced, 5);

	// Cancelled, it stops, and its body is dropped.
	send(&mut socket, ClientFrame::Cancel { stream: 1 }).await;
	for _ in 0..100 {
		if progress(&run, |_| {}).dropped {
			break;
		}

		tokio::time::sleep(Duration::from_millis(10)).await;
	}
	assert!(
		progress(&run, |_| {}).dropped,
		"the cancelled body is dropped"
	);
	send(&mut socket, ClientFrame::Credit { stream: 1, n: 10 }).await;
	assert!(
		next_within(&mut socket, Duration::from_millis(200))
			.await
			.is_none(),
		"a cancelled stream sends nothing"
	);

	// One with room to spare ends by itself.
	let done = run_name("done");
	send(
		&mut socket,
		open(2, "l6_count", json!({ "run": done, "upto": 2 }), 10),
	)
	.await;
	assert_eq!(item(&next(&mut socket).await), Some((2, &json!(0))));
	assert_eq!(item(&next(&mut socket).await), Some((2, &json!(1))));
	assert_eq!(end(&next(&mut socket).await), Some((2, EndReason::Done)));
}

/// A failing item ends the stream as failed, a refusal fails it at once,
/// and an unknown stream is refused.
async fn streams_fail(db: Db) {
	let server = serve(db).await;
	let mut socket = connect(&server).await;

	send(
		&mut socket,
		open(1, "l6_flaky", json!({ "refuse": false }), 10),
	)
	.await;
	assert_eq!(item(&next(&mut socket).await), Some((1, &json!(1))));
	let Some((1, EndReason::Failed(error))) = end(&next(&mut socket).await) else {
		panic!("the failure ends it");
	};
	assert_eq!(error.status, 409);
	assert_eq!(error.message, "the stream broke");

	send(
		&mut socket,
		open(2, "l6_flaky", json!({ "refuse": true }), 10),
	)
	.await;
	let Some((2, EndReason::Failed(error))) = end(&next(&mut socket).await) else {
		panic!("the refusal ends it");
	};
	assert_eq!(error.status, 403);

	send(&mut socket, open(3, "nothing_here", json!({}), 10)).await;
	let Some((3, EndReason::Failed(error))) = end(&next(&mut socket).await) else {
		panic!("an unknown stream fails");
	};
	assert!(error.message.contains("no stream is named"));

	send(
		&mut socket,
		open(4, "l6_flaky", json!({ "refuse": "no" }), 10),
	)
	.await;
	let Some((4, EndReason::Failed(error))) = end(&next(&mut socket).await) else {
		panic!("wrong arguments fail");
	};
	assert_eq!(error.status, 400);
}

/// The same stream, credits and all, over server-sent events.
async fn streams_over_server_sent_events(db: Db) {
	let run = run_name("events");
	let server = serve(db).await;
	let mut events = Events::open(&server).await;
	assert_eq!(
		events
			.post(
				&server,
				&[open(1, "l6_count", json!({ "run": run, "upto": 3 }), 2)]
			)
			.await,
		StatusCode::ACCEPTED
	);
	assert_eq!(item(&events.next().await), Some((1, &json!(0))));
	assert_eq!(item(&events.next().await), Some((1, &json!(1))));
	assert_eq!(progress(&run, |_| {}).produced, 2);
	events
		.post(&server, &[ClientFrame::Credit { stream: 1, n: 5 }])
		.await;
	assert_eq!(item(&events.next().await), Some((1, &json!(2))));
	assert_eq!(end(&events.next().await), Some((1, EndReason::Done)));
}

/// Server-sent events, read in process.
struct Events {
	body: Body,
	buffer: String,
	connection: String,
}

impl Events {
	async fn open(server: &Server) -> Self {
		let response = server
			.app
			.clone()
			.oneshot(Request::get("/live/events").body(Body::empty()).unwrap())
			.await
			.unwrap();
		assert_eq!(response.status(), StatusCode::OK);
		let mut events = Self {
			body: response.into_body(),
			buffer: String::new(),
			connection: String::new(),
		};
		let ServerFrame::Welcome { connection, .. } = events.next().await else {
			panic!("welcome first");
		};
		events.connection = connection;
		events
	}

	async fn next(&mut self) -> ServerFrame {
		loop {
			if let Some(end) = self.buffer.find("\n\n") {
				let event: String = self.buffer.drain(..end + 2).collect();
				let data: String = event
					.lines()
					.filter_map(|line| line.strip_prefix("data:"))
					.map(str::trim_start)
					.collect();

				if data.is_empty() {
					continue;
				}

				return serde_json::from_str(&data).unwrap();
			}

			let frame = tokio::time::timeout(Duration::from_secs(20), self.body.frame())
				.await
				.expect("an event arrives")
				.expect("the stream stays open")
				.unwrap();

			if let Ok(bytes) = frame.into_data() {
				self.buffer.push_str(std::str::from_utf8(&bytes).unwrap());
			}
		}
	}

	async fn post(&self, server: &Server, frames: &[ClientFrame]) -> StatusCode {
		server
			.app
			.clone()
			.oneshot(
				Request::post(format!("/live/frames?connection={}", self.connection))
					.header("content-type", "application/json")
					.body(Body::from(serde_json::to_vec(frames).unwrap()))
					.unwrap(),
			)
			.await
			.unwrap()
			.status()
	}
}

// ---- pages ---------------------------------------------------------------

async fn post(db: &Db, channel: &str, body: &str) -> Id<Post> {
	NewPost {
		channel: channel.into(),
		body: body.into(),
	}
	.insert(db)
	.await
	.unwrap()
	.id
}

fn page_args(after: Option<&str>, until: Option<&str>, limit: u32) -> Value {
	let mut page = json!({ "limit": limit });

	if let Some(after) = after {
		page["after"] = json!(after);
	}

	if let Some(until) = until {
		page["until"] = json!(until);
	}

	json!({ "channel": "general", "page": page })
}

async fn subscribe(socket: &mut Socket, id: u32, query: &str, args: Value) -> Value {
	send(
		socket,
		ClientFrame::Sub {
			id,
			query: query.into(),
			args,
		},
	)
	.await;

	loop {
		match next(socket).await {
			ServerFrame::Done {
				request,
				ok: Some(value),
				..
			} if request == id => return value,
			// The connection catching up with the commits made before it
			// subscribed.
			ServerFrame::Transition(_) => {}
			other => panic!("expected the first value, got {other:?}"),
		}
	}
}

fn bodies(page: &Value) -> Vec<String> {
	page["items"]
		.as_array()
		.unwrap()
		.iter()
		.map(|row| row["body"].as_str().unwrap().to_owned())
		.collect()
}

async fn transition(socket: &mut Socket) -> Transition {
	loop {
		if let ServerFrame::Transition(transition) = next(socket).await {
			return transition;
		}
	}
}

/// The next transition that changes `id`, applied to `value`.
async fn changed(socket: &mut Socket, id: u32, value: &mut Value) -> Transition {
	loop {
		let seen = transition(socket).await;

		if let Some(update) = seen.updates.iter().find(|update| update.id == id) {
			rpc::live::apply(value, &update.update);
			return seen;
		}
	}
}

/// Pages pinned at their ends: an insert into page one lengthens it and
/// leaves page two alone, a delete shortens its own page, and every page
/// keeps updating.
async fn pages_stay_put(db: Db) {
	let server = serve(db).await;
	let mut socket = connect(&server).await;
	let mut ids = Vec::new();

	for body in ["p1", "p2", "p3", "p4", "p5"] {
		ids.push(post(&server.db, "general", body).await);
	}

	post(&server.db, "elsewhere", "other").await;

	// The first page, newest first.
	let first = subscribe(&mut socket, 1, "l6_history", page_args(None, None, 2)).await;
	assert_eq!(bodies(&first), ["p5", "p4"]);
	assert_eq!(first["has_more"], json!(true));
	let end = first["end"].as_str().unwrap().to_owned();

	// Loading more pins the first page at its end, and reads after it.
	send(&mut socket, ClientFrame::Unsub { id: 1 }).await;
	let mut pinned = subscribe(&mut socket, 2, "l6_history", page_args(None, Some(&end), 2)).await;
	assert_eq!(bodies(&pinned), ["p5", "p4"]);
	let mut second = subscribe(&mut socket, 3, "l6_history", page_args(Some(&end), None, 2)).await;
	assert_eq!(bodies(&second), ["p3", "p2"]);
	assert_eq!(second["has_more"], json!(true));

	// A new post lands on page one, which grows, as a patch of its items;
	// page two is untouched.
	post(&server.db, "general", "p6").await;
	let seen = changed(&mut socket, 2, &mut pinned).await;
	assert_eq!(bodies(&pinned), ["p6", "p5", "p4"]);
	assert!(
		seen.updates.iter().all(|update| update.id != 3),
		"page two didn't move"
	);
	let update = &seen
		.updates
		.iter()
		.find(|update| update.id == 2)
		.unwrap()
		.update;
	assert!(
		matches!(update, Update::Patch(patch) if patch.field.as_deref() == Some("items")),
		"{update:?}"
	);

	// Deleting from page one shortens it, and page two stays.
	Post::query()
		.filter(|post| post.id.eq(ids[4]))
		.delete(&server.db)
		.await
		.unwrap();
	changed(&mut socket, 2, &mut pinned).await;
	assert_eq!(bodies(&pinned), ["p6", "p4"]);
	assert_eq!(bodies(&second), ["p3", "p2"]);

	// Deleting from the last page pulls the next row in.
	Post::query()
		.filter(|post| post.id.eq(ids[2]))
		.delete(&server.db)
		.await
		.unwrap();
	changed(&mut socket, 3, &mut second).await;
	assert_eq!(bodies(&second), ["p2", "p1"]);
	assert_eq!(second["has_more"], json!(false));
	assert_eq!(bodies(&pinned), ["p6", "p4"]);

	// A cursor this column never made is refused.
	send(
		&mut socket,
		ClientFrame::Sub {
			id: 4,
			query: "l6_history".into(),
			args: page_args(Some("not a cursor"), None, 2),
		},
	)
	.await;
	let error = loop {
		match next(&mut socket).await {
			ServerFrame::Done {
				err: Some(error), ..
			} => break error,
			ServerFrame::Transition(_) => {}
			other => panic!("a bad cursor fails, not {other:?}"),
		}
	};
	assert_eq!(error.status, 400);
}

// ---- the client ----------------------------------------------------------

static START: Once = Once::new();

fn start() {
	START.call_once(|| {
		any_spawner::Executor::init_tokio().unwrap();
	});
}

fn live(db: &Db) -> Live {
	Live::new(db.clone()).identify(Header)
}

async fn wait_for(what: &str, mut ready: impl FnMut() -> bool) {
	for _ in 0..1_000 {
		if ready() {
			return;
		}

		tokio::time::sleep(Duration::from_millis(10)).await;
	}

	panic!("waited too long for {what}");
}

async fn next_update(updates: &mut futures::channel::mpsc::UnboundedReceiver<Update>) -> Value {
	loop {
		let update = tokio::time::timeout(Duration::from_secs(20), updates.next())
			.await
			.expect("an update arrives")
			.expect("the subscription stays");

		if let Update::Ok(value) = update {
			return value;
		}
	}
}

/// Waits for a subscription's value to be `expected`, applying patches.
async fn until_value(
	updates: &mut futures::channel::mpsc::UnboundedReceiver<Update>,
	expected: &Value,
) {
	let mut value = Value::Null;

	while &value != expected {
		let update = tokio::time::timeout(Duration::from_secs(20), updates.next())
			.await
			.unwrap_or_else(|_| panic!("waited for {expected}, have {value}"))
			.expect("the subscription stays");
		rpc::live::apply(&mut value, &update);
	}
}

async fn note_bodies(db: &Db) -> Vec<String> {
	Note::query()
		.order_by(|note| note.id.asc())
		.all(db)
		.await
		.unwrap()
		.into_iter()
		.map(|note| note.body)
		.collect()
}

/// Calls dispatched while the network is down wait in order, are kept on
/// the device, and run once each when it comes back.
async fn the_offline_queue(db: Db) {
	start();
	let server = live(&db);
	let clients = LiveClients::new(&server);
	let queue = MemoryQueue::new();

	LocalSet::new()
		.run_until(async {
			let client = clients.client(&ClientConfig::new().offline_store(queue.clone()));
			wait_for("the connection", || client.status().is_live()).await;
			assert!(client.carries_calls());

			clients.outage(true);
			wait_for("the outage", || !client.status().is_live()).await;
			let first = client.mutate("l6_write", json!({ "body": "one" }), Some("k-one".into()));
			let second = client.act("l6_write_later", json!({ "body": "two" }));
			let third = client.mutate(
				"l6_write",
				json!({ "body": "three" }),
				Some("k-three".into()),
			);
			tokio::time::sleep(Duration::from_millis(50)).await;
			assert_eq!(client.status().queued, 3);
			assert_eq!(
				queue
					.calls()
					.iter()
					.map(|call| call.kind)
					.collect::<Vec<_>>(),
				[
					QueuedKind::Mutation,
					QueuedKind::Action,
					QueuedKind::Mutation
				],
				"kept on the device, in order"
			);
			assert_eq!(note_bodies(&db).await, [] as [String; 0]);

			clients.outage(false);
			let (first, second, third) = futures::join!(first, second, third);
			first.unwrap();
			second.unwrap();
			third.unwrap();
			// Sent in order; mutations also run in order, and an action runs
			// in its own task beside them.
			let mut bodies = note_bodies(&db).await;
			let one = bodies.iter().position(|body| body == "one");
			let three = bodies.iter().position(|body| body == "three");
			assert!(one < three, "{bodies:?}");
			bodies.sort();
			assert_eq!(bodies, ["one", "three", "two"], "once each");
			assert_eq!(client.status().queued, 0);
			wait_for("the device's copy to empty", || queue.calls().is_empty()).await;

			// Dropped and back again: nothing runs twice.
			clients.outage(false);
			wait_for("the reconnect", || client.status().is_live()).await;
			tokio::time::sleep(Duration::from_millis(100)).await;
			assert_eq!(note_bodies(&db).await.len(), 3);
		})
		.await;
}

/// What the device kept goes out when the app starts again: a mutation
/// whose first send went through runs once, by its key, and an action that
/// never went out runs.
async fn replaying_after_a_restart(db: Db) {
	start();
	let server = live(&db);
	let clients = LiveClients::new(&server);
	let queue = MemoryQueue::new();

	// The app dispatched while offline, and closed.
	clients.outage(true);
	LocalSet::new()
		.run_until(async {
			let client = clients.client(&ClientConfig::new().offline_store(queue.clone()));
			let _pending =
				client.mutate("l6_write", json!({ "body": "kept" }), Some("k-kept".into()));
			wait_for("the device's copy", || queue.calls().len() == 1).await;
		})
		.await;
	clients.outage(false);

	// Its first send had reached the server before it closed.
	rpc::server::call_as(
		&db,
		Identity::anonymous(),
		Some("k-kept"),
		L6Write {
			body: "kept".into(),
		},
	)
	.await
	.unwrap();
	queue
		.save(vec![
			queue.calls()[0].clone(),
			QueuedCall {
				kind: QueuedKind::Action,
				name: "l6_write_later".into(),
				args: json!({ "body": "never sent" }),
				key: None,
			},
		])
		.await;

	LocalSet::new()
		.run_until(async {
			let client = clients.client(&ClientConfig::new().offline_store(queue.clone()));
			wait_for("the replay", || {
				client.status().is_live() && client.status().queued == 0
			})
			.await;
			assert_eq!(note_bodies(&db).await, ["kept", "never sent"]);
			assert_eq!(queue.calls(), [] as [QueuedCall; 0]);
		})
		.await;
}

/// Bearer credentials the test signs in and out.
#[derive(Clone, Default)]
struct Token(Arc<Mutex<Option<String>>>);

impl Token {
	fn set(&self, token: Option<&str>) {
		*self.0.lock().unwrap() = token.map(str::to_owned);
	}
}

impl Credentials for Token {
	fn authorization(&self) -> BoxFuture<'_, Result<Option<String>, FnError>> {
		let header = self
			.0
			.lock()
			.unwrap()
			.as_ref()
			.map(|token| format!("Bearer {token}"));
		Box::pin(std::future::ready(Ok(header)))
	}
}

/// Signing in and out on an open connection: private results appear, and
/// vanish, without a reload, by token in place and by cookie by
/// reconnecting.
async fn reauthenticating(db: Db) {
	start();
	NewNote {
		owner: Some("ada".into()),
		body: "ada's".into(),
	}
	.insert(&db)
	.await
	.unwrap();
	let server = live(&db);
	let clients = LiveClients::new(&server);

	LocalSet::new()
		.run_until(async {
			// A native app, with a bearer token.
			let token = Token::default();
			let client: LiveClient =
				clients.client(&ClientConfig::new().credentials(token.clone()));
			let (_, mut mine) = client.subscribe("l6_mine", json!({}));
			assert_eq!(next_update(&mut mine).await, json!([]));
			let connections = server.stats().connections;

			token.set(Some("ada"));
			client.reauthenticate();
			until_value(&mut mine, &json!(["ada's"])).await;
			assert_eq!(server.stats().connections, connections, "in place");

			token.set(None);
			client.reauthenticate();
			until_value(&mut mine, &json!([])).await;

			// A browser, with a cookie.
			let browser = clients.client(&ClientConfig::new());
			let (_, mut theirs) = browser.subscribe("l6_mine", json!({}));
			assert_eq!(next_update(&mut theirs).await, json!([]));
			clients.header("x-caller", Some("ada"));
			browser.reauthenticate();
			until_value(&mut theirs, &json!(["ada's"])).await;
			clients.header("x-caller", None);
			browser.reauthenticate();
			until_value(&mut theirs, &json!([])).await;
		})
		.await;
}

/// A resumable stream opens again from its last item after a reconnect;
/// any other ends as interrupted.
async fn streams_through_the_client(db: Db) {
	start();
	let run = run_name("ticks");
	let server = live(&db);
	let clients = LiveClients::new(&server);

	LocalSet::new()
		.run_until(async {
			let client = clients.client(&ClientConfig::new());
			let mut ticks =
				client.stream("l6_ticks", json!({ "run": run, "cursor": null }), 4, true);
			let mut seen = Vec::new();

			while seen.len() < 3 {
				seen.push(tick(&mut ticks).await);
			}

			clients.outage(false);

			while seen.len() < 8 {
				seen.push(tick(&mut ticks).await);
			}

			assert_eq!(seen, (0..8).collect::<Vec<_>>(), "no gap and no repeat");
			let opened = progress(&run, |_| {}).opened_at;
			assert_eq!(opened.first(), Some(&None));
			assert!(opened.len() >= 2, "opened again: {opened:?}");
			assert!(opened[1].is_some(), "from a cursor");

			let count = run_name("count");
			let mut count =
				client.stream("l6_count", json!({ "run": count, "upto": 1000 }), 2, false);
			assert!(matches!(
				tokio::time::timeout(Duration::from_secs(20), count.next()).await,
				Ok(Some(StreamEvent::Item(_)))
			));
			clients.outage(false);
			let mut ended = None;

			while ended.is_none() {
				match tokio::time::timeout(Duration::from_secs(20), count.next())
					.await
					.unwrap()
				{
					Some(StreamEvent::End(reason)) => ended = Some(reason),
					Some(StreamEvent::Item(_)) => {}
					None => panic!("it ends with a reason"),
				}
			}

			assert_eq!(ended, Some(EndReason::Interrupted));
		})
		.await;
}

async fn tick(ticks: &mut rpc::client::LiveStream) -> u32 {
	match tokio::time::timeout(Duration::from_secs(20), ticks.next())
		.await
		.expect("a tick arrives")
	{
		Some(StreamEvent::Item(value)) => serde_json::from_value::<Tick>(value).unwrap().n,
		other => panic!("expected a tick, got {other:?}"),
	}
}

macro_rules! on_sqlite {
	($($name:ident),* $(,)?) => {
		mod sqlite {
			$(
				#[tokio::test(flavor = "multi_thread")]
				async fn $name() {
					super::$name(leptoskit_db::testing::sqlite().await).await;
				}
			)*
		}
	};
}

on_sqlite!(
	streams_wait_for_credits,
	streams_fail,
	streams_over_server_sent_events,
	pages_stay_put,
	the_offline_queue,
	replaying_after_a_restart,
	reauthenticating,
	streams_through_the_client,
);

#[tokio::test(flavor = "multi_thread")]
async fn on_postgres() {
	let Some(cluster) = PostgresCluster::start() else {
		return;
	};
	let fresh = async || {
		let db = Db::connect(&cluster.database()).await.unwrap();
		leptoskit_db::testing::create_tables(&db).await.unwrap();
		db
	};

	streams_wait_for_credits(fresh().await).await;
	streams_fail(fresh().await).await;
	streams_over_server_sent_events(fresh().await).await;
	pages_stay_put(fresh().await).await;
	the_offline_queue(fresh().await).await;
	replaying_after_a_restart(fresh().await).await;
	reauthenticating(fresh().await).await;
	streams_through_the_client(fresh().await).await;
}
