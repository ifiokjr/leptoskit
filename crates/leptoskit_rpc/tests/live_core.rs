//! The live core on SQLite and Postgres: one hub per server that reads each
//! commit once, re-runs shared between connections, read-your-writes from
//! the commit's own version, resuming from a version, actions over the
//! connection, and the same session over server-sent events.

#![cfg(all(feature = "live", feature = "postgres", feature = "sqlite"))]

#[path = "../../leptoskit_db/tests/support/postgres.rs"]
mod postgres;

use std::pin::Pin;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use futures::SinkExt;
use futures::StreamExt;
use http::Request;
use http::StatusCode;
use http_body_util::BodyExt;
use leptoskit_db::Db;
use leptoskit_db::Id;
use leptoskit_db::table;
use leptoskit_rpc as rpc;
use leptoskit_rpc::live::ClientFrame;
use leptoskit_rpc::live::Resume;
use leptoskit_rpc::live::ResumeQuery;
use leptoskit_rpc::live::ServerFrame;
use leptoskit_rpc::live::Transition;
use leptoskit_rpc::live::Update;
use leptoskit_rpc::server::Identity;
use leptoskit_rpc::server::Live;
use postgres::PostgresCluster;
use serde_json::Value;
use serde_json::json;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use tokio_tungstenite::MaybeTlsStream;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tower::ServiceExt;

/// A note everyone may read.
#[table(name = "core_notes", live)]
pub struct Note {
	pub id: Id<Self>,
	pub body: String,
}

/// A tag, in its own live table.
#[table(name = "core_tags", live)]
pub struct Tag {
	pub id: Id<Self>,
	pub name: String,
}

/// Every note's body, in order.
#[rpc::query]
pub async fn core_titles(cx: &rpc::QueryCx) -> rpc::Result<Vec<String>> {
	let notes = Note::query()
		.order_by(|note| note.body.asc())
		.all(cx.db())
		.await?;
	Ok(notes.into_iter().map(|note| note.body).collect())
}

/// Every tag's name, in order.
#[rpc::query]
pub async fn core_tags(cx: &rpc::QueryCx) -> rpc::Result<Vec<String>> {
	let tags = Tag::query()
		.order_by(|tag| tag.name.asc())
		.all(cx.db())
		.await?;
	Ok(tags.into_iter().map(|tag| tag.name).collect())
}

/// Adds a note.
#[rpc::mutation]
pub async fn add_core_note(cx: &rpc::MutationCx, body: String) -> rpc::Result<String> {
	let note = NewNote { body }.insert(cx.tx()).await?;
	Ok(note.id.to_string())
}

/// Adds a tag.
#[rpc::mutation]
pub async fn add_core_tag(cx: &rpc::MutationCx, name: String) -> rpc::Result<()> {
	NewTag { name }.insert(cx.tx()).await?;
	Ok(())
}

/// Who the action runs as, and a note it adds through a mutation.
#[rpc::action]
pub async fn note_as_caller(cx: &rpc::ActionCx, body: String) -> rpc::Result<String> {
	let caller = cx
		.identity()
		.caller
		.clone()
		.ok_or_else(|| rpc::Error::unauthorized("say who you are"))?;
	cx.mutate(AddCoreNote {
		body: format!("{caller}: {body}"),
	})
	.await?;
	Ok(caller)
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

struct Server {
	db: Db,
	live: Live,
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
		live,
		app,
		url: format!("ws://{address}/live"),
		task,
	}
}

type Socket = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

/// A WebSocket, after its welcome, with the welcome's version.
async fn connect(server: &Server, caller: Option<&str>) -> (Socket, u64) {
	let mut request = server.url.as_str().into_client_request().unwrap();

	if let Some(caller) = caller {
		request
			.headers_mut()
			.insert("x-caller", caller.parse().unwrap());
	}

	let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
	let ServerFrame::Welcome { version, .. } = next(&mut socket).await else {
		panic!("welcome first");
	};
	(socket, version)
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

async fn transition(socket: &mut Socket) -> Transition {
	match next(socket).await {
		ServerFrame::Transition(transition) => transition,
		other => panic!("expected a transition, got {other:?}"),
	}
}

async fn subscribe(socket: &mut Socket, id: u32, query: &str) -> Value {
	send(
		socket,
		ClientFrame::Sub {
			id,
			query: query.into(),
			args: json!({}),
		},
	)
	.await;

	match next(socket).await {
		ServerFrame::Done {
			request,
			ok: Some(value),
			..
		} if request == id => value,
		other => panic!("expected the subscription's first value, got {other:?}"),
	}
}

fn whole(transition: &Transition, id: u32) -> Option<&Value> {
	transition
		.updates
		.iter()
		.find(|update| update.id == id)
		.and_then(|update| {
			match &update.update {
				Update::Ok(value) => Some(value),
				_ => None,
			}
		})
}

async fn newest(db: &Db) -> u64 {
	u64::try_from(db.live_version().await.unwrap()).unwrap()
}

async fn note(db: &Db, body: &str) {
	NewNote { body: body.into() }.insert(db).await.unwrap();
}

async fn tag(db: &Db, name: &str) {
	NewTag { name: name.into() }.insert(db).await.unwrap();
}

/// Subscriptions to the same query by the same caller share one re-run per
/// commit, on however many connections.
async fn connections_share_one_rerun(db: Db) {
	let server = serve(db).await;
	let (mut ada, _) = connect(&server, None).await;
	let (mut grace, _) = connect(&server, None).await;
	assert_eq!(subscribe(&mut ada, 1, "core_titles").await, json!([]));
	assert_eq!(subscribe(&mut grace, 7, "core_titles").await, json!([]));

	let before = server.live.stats();
	note(&server.db, "first").await;
	let seen = transition(&mut ada).await;
	let also = transition(&mut grace).await;
	assert_eq!(seen.updates[0].id, 1);
	assert_eq!(also.updates[0].id, 7);
	assert_eq!(seen.updates[0].update, also.updates[0].update, "one diff");
	let after = server.live.stats();
	assert_eq!(after.reruns - before.reruns, 1, "one re-run for both");
	assert_eq!(after.commits - before.commits, 1);

	// A different caller reads as someone else: its own re-run.
	let (mut someone, _) = connect(&server, Some("someone")).await;
	assert_eq!(
		subscribe(&mut someone, 1, "core_titles").await,
		json!(["first"])
	);
	let before = server.live.stats();
	note(&server.db, "second").await;
	transition(&mut ada).await;
	transition(&mut grace).await;
	transition(&mut someone).await;
	assert_eq!(server.live.stats().reruns - before.reruns, 2);
}

/// One reader follows the log for the whole server: a commit is read once
/// however many connections watch it.
async fn one_log_read_per_commit(db: Db) {
	let server = serve(db).await;
	let mut sockets = Vec::new();

	for count in [1, 6] {
		while sockets.len() < count {
			let (mut socket, _) = connect(&server, None).await;
			subscribe(&mut socket, 1, "core_tags").await;
			sockets.push(socket);
		}

		let before = server.live.stats();
		tag(&server.db, &format!("tag-{count}")).await;

		for socket in &mut sockets {
			let seen = transition(socket).await;
			assert_eq!(seen.to, newest(&server.db).await);
		}

		let after = server.live.stats();
		assert_eq!(
			after.commits - before.commits,
			1,
			"{count} connections read the commit once"
		);
		assert_eq!(after.reruns - before.reruns, 1);
	}
}

/// A mutation's `done` names its commit, and a transition reaching it
/// follows, whether or not the commit changed the caller's results.
async fn read_your_writes(db: Db) {
	let server = serve(db).await;
	let (mut socket, _) = connect(&server, None).await;
	subscribe(&mut socket, 1, "core_titles").await;

	send(
		&mut socket,
		ClientFrame::Mutate {
			request: 2,
			name: "add_core_note".into(),
			args: json!({ "body": "mine" }),
			key: Some("note-1".into()),
		},
	)
	.await;
	let ServerFrame::Done {
		request,
		err,
		version,
		..
	} = next(&mut socket).await
	else {
		panic!("the mutation answers first");
	};
	assert_eq!((request, err), (2, None));
	assert_eq!(
		version,
		newest(&server.db).await,
		"the commit's own version"
	);
	let seen = transition(&mut socket).await;
	assert!(seen.to >= version);
	assert_eq!(seen.updates.len(), 1);

	let reached = a_write_the_connection_watches_nothing_of(&server, &mut socket).await;
	a_repeat_answers_from_its_receipt(&server, &mut socket, reached).await;
}

async fn a_write_the_connection_watches_nothing_of(server: &Server, socket: &mut Socket) -> u64 {
	// A write the connection watches nothing of still reaches it, empty.
	send(
		socket,
		ClientFrame::Mutate {
			request: 3,
			name: "add_core_tag".into(),
			args: json!({ "name": "elsewhere" }),
			key: None,
		},
	)
	.await;
	let ServerFrame::Done { version, err, .. } = next(socket).await else {
		panic!("the mutation answers");
	};
	assert_eq!(err, None);
	assert_eq!(version, newest(&server.db).await);
	let seen = transition(socket).await;
	assert!(seen.to >= version, "{} < {version}", seen.to);
	assert_eq!(
		seen.updates,
		[] as [leptoskit_rpc::live::SubscriptionUpdate; 0]
	);

	seen.to
}

async fn a_repeat_answers_from_its_receipt(server: &Server, socket: &mut Socket, reached: u64) {
	// The same key again: the receipt answers, at a version already reached.
	send(
		socket,
		ClientFrame::Mutate {
			request: 4,
			name: "add_core_note".into(),
			args: json!({ "body": "mine" }),
			key: Some("note-1".into()),
		},
	)
	.await;
	let ServerFrame::Done { version, err, .. } = next(socket).await else {
		panic!("the repeat answers");
	};
	assert_eq!(err, None);
	assert!(version <= reached);
	assert_eq!(
		Note::query().all(&server.db).await.unwrap().len(),
		1,
		"the repeat ran once"
	);
}

/// Actions run over the connection as its caller, and answer with the
/// version their mutations committed at.
async fn actions_over_the_connection(db: Db) {
	let server = serve(db).await;
	let (mut socket, _) = connect(&server, Some("ada")).await;
	subscribe(&mut socket, 1, "core_titles").await;

	send(
		&mut socket,
		ClientFrame::Act {
			request: 2,
			name: "note_as_caller".into(),
			args: json!({ "body": "hello" }),
		},
	)
	.await;
	let ServerFrame::Done {
		request,
		ok,
		err,
		version,
	} = next(&mut socket).await
	else {
		panic!("the action answers");
	};
	assert_eq!((request, err), (2, None));
	assert_eq!(ok, Some(json!("ada")));
	assert_eq!(version, newest(&server.db).await);
	let seen = transition(&mut socket).await;
	assert!(seen.to >= version);

	// The same refusals as over HTTP.
	let (mut nobody, _) = connect(&server, None).await;
	send(
		&mut nobody,
		ClientFrame::Act {
			request: 5,
			name: "note_as_caller".into(),
			args: json!({ "body": "hello" }),
		},
	)
	.await;
	let ServerFrame::Done { err, .. } = next(&mut nobody).await else {
		panic!("the refusal");
	};
	assert_eq!(err.unwrap().status, 401);

	send(
		&mut nobody,
		ClientFrame::Act {
			request: 6,
			name: "add_core_tag".into(),
			args: json!({ "name": "x" }),
		},
	)
	.await;
	let ServerFrame::Done { err, .. } = next(&mut nobody).await else {
		panic!("the refusal");
	};
	assert!(err.unwrap().message.contains("only an action"));
}

fn resume(version: u64) -> ClientFrame {
	ClientFrame::Hello {
		resume: Some(Resume {
			version,
			queries: vec![
				ResumeQuery {
					id: 1,
					query: "core_titles".into(),
					args: json!({}),
				},
				ResumeQuery {
					id: 2,
					query: "core_tags".into(),
					args: json!({}),
				},
			],
		}),
	}
}

/// A reconnecting client sends what it holds and the version it reached,
/// and hears only what changed since; once the log is pruned past that
/// version, it hears everything.
async fn resuming(db: Db) {
	let server = serve(db).await;
	let (mut socket, _) = connect(&server, None).await;
	subscribe(&mut socket, 1, "core_titles").await;
	subscribe(&mut socket, 2, "core_tags").await;
	tag(&server.db, "before").await;
	let reached = transition(&mut socket).await.to;
	drop(socket);

	note(&server.db, "while away").await;
	let before = server.live.stats();
	let (mut socket, _) = connect(&server, None).await;
	send(&mut socket, resume(reached)).await;
	let caught_up = transition(&mut socket).await;
	assert_eq!(caught_up.from, reached);
	assert!(!caught_up.full);
	assert!(whole(&caught_up, 2).is_none(), "the tags didn't change");
	// The hub may not have read the note's commit when the client came
	// back; then the change follows in the next transition.
	let notes = match whole(&caught_up, 1) {
		Some(notes) => notes.clone(),
		None => whole(&transition(&mut socket).await, 1).unwrap().clone(),
	};
	assert_eq!(notes, json!(["while away"]));
	assert_eq!(server.live.stats().resumes - before.resumes, 1);

	// Both subscriptions are live again.
	tag(&server.db, "after").await;
	let seen = transition(&mut socket).await;
	assert_eq!(whole(&seen, 2), Some(&json!(["after", "before"])));
	drop(socket);

	resuming_after_pruning(&server, reached).await;
}

async fn resuming_after_pruning(server: &Server, reached: u64) {
	// Pruned past where the client was: every result again.
	server
		.db
		.system("pruning")
		.prune_changes(Duration::ZERO)
		.await
		.unwrap();
	let before = server.live.stats();
	let (mut socket, _) = connect(server, None).await;
	send(&mut socket, resume(reached)).await;
	let everything = transition(&mut socket).await;
	assert!(everything.full);
	assert_eq!(everything.updates.len(), 2);
	assert_eq!(whole(&everything, 1), Some(&json!(["while away"])));
	assert_eq!(whole(&everything, 2), Some(&json!(["after", "before"])));
	assert_eq!(server.live.stats().resumes, before.resumes);
}

/// Server-sent events, read in process.
struct Events {
	body: Body,
	buffer: String,
	connection: String,
}

impl Events {
	async fn open(server: &Server, headers: &[(&str, &str)]) -> (Self, u64) {
		let mut request = Request::get("/live/events");

		for (name, value) in headers {
			request = request.header(*name, *value);
		}

		let response = server
			.app
			.clone()
			.oneshot(request.body(Body::empty()).unwrap())
			.await
			.unwrap();
		assert_eq!(response.status(), StatusCode::OK);
		assert_eq!(
			response.headers()["content-type"],
			"text/event-stream",
			"an event stream"
		);
		let mut events = Self {
			body: response.into_body(),
			buffer: String::new(),
			connection: String::new(),
		};
		let (
			_,
			ServerFrame::Welcome {
				connection,
				version,
				..
			},
		) = events.next().await
		else {
			panic!("welcome first");
		};
		events.connection = connection;
		(events, version)
	}

	/// The next event, with its id.
	async fn next(&mut self) -> (Option<u64>, ServerFrame) {
		loop {
			if let Some(end) = self.buffer.find("\n\n") {
				let event: String = self.buffer.drain(..end + 2).collect();
				let mut id = None;
				let mut data = String::new();

				for line in event.lines() {
					if let Some(value) = line.strip_prefix("id:") {
						id = Some(value.trim().parse().unwrap());
					} else if let Some(value) = line.strip_prefix("data:") {
						data.push_str(value.trim_start());
					}
				}

				if data.is_empty() {
					// A heartbeat.
					continue;
				}

				return (id, serde_json::from_str(&data).unwrap());
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

	async fn post(
		&self,
		server: &Server,
		frames: &[ClientFrame],
		headers: &[(&str, &str)],
	) -> StatusCode {
		let mut request = Request::post(format!("/live/frames?connection={}", self.connection))
			.header("content-type", "application/json");

		for (name, value) in headers {
			request = request.header(*name, *value);
		}

		server
			.app
			.clone()
			.oneshot(
				request
					.body(Body::from(serde_json::to_vec(frames).unwrap()))
					.unwrap(),
			)
			.await
			.unwrap()
			.status()
	}
}

/// The same session over server-sent events: frames posted, answers and
/// transitions streamed, each transition's version as its event id.
async fn server_sent_events(db: Db) {
	let server = serve(db).await;
	let (mut events, version) = Events::open(&server, &[]).await;
	assert_eq!(version, newest(&server.db).await);

	let subscribe = |id: u32, query: &str| {
		ClientFrame::Sub {
			id,
			query: query.into(),
			args: json!({}),
		}
	};
	assert_eq!(
		events
			.post(
				&server,
				&[subscribe(1, "core_titles"), subscribe(2, "core_tags")],
				&[]
			)
			.await,
		StatusCode::ACCEPTED
	);
	for id in [1, 2] {
		let (event_id, ServerFrame::Done { request, ok, .. }) = events.next().await else {
			panic!("the subscription's first value");
		};
		assert_eq!((event_id, request, ok), (None, id, Some(json!([]))));
	}

	note(&server.db, "over events").await;
	let (id, ServerFrame::Transition(seen)) = events.next().await else {
		panic!("the write arrives");
	};
	assert_eq!(id, Some(seen.to));
	assert_eq!(seen.to, newest(&server.db).await);

	events
		.post(
			&server,
			&[ClientFrame::Mutate {
				request: 3,
				name: "add_core_tag".into(),
				args: json!({ "name": "posted" }),
				key: None,
			}],
			&[],
		)
		.await;
	let (_, ServerFrame::Done { version, err, .. }) = events.next().await else {
		panic!("the mutation answers");
	};
	assert_eq!(err, None);
	let (id, ServerFrame::Transition(seen)) = events.next().await else {
		panic!("the mutation's write arrives");
	};
	assert!(seen.to >= version);
	assert_eq!(id, Some(seen.to));
	let reached = seen.to;
	drop(events);

	let events = resuming_from_the_last_event(&server, reached).await;
	refusing_posts(&server, &events).await;
}

async fn resuming_from_the_last_event(server: &Server, reached: u64) -> Events {
	// Reconnecting with `Last-Event-ID` resumes from it.
	tag(&server.db, "while away").await;
	let (events, _) = Events::open(server, &[("last-event-id", &reached.to_string())]).await;
	let mut events = events;
	events.post(server, &[resume(0)], &[]).await;
	let (_, ServerFrame::Transition(caught_up)) = events.next().await else {
		panic!("the resume");
	};
	assert_eq!(caught_up.from, reached);
	assert!(!caught_up.full);
	assert!(whole(&caught_up, 1).is_none(), "the notes didn't change");
	let tags = if let Some(tags) = whole(&caught_up, 2) {
		tags.clone()
	} else {
		let (_, ServerFrame::Transition(next)) = events.next().await else {
			panic!("the tag's change");
		};
		whole(&next, 2).unwrap().clone()
	};
	assert_eq!(tags, json!(["posted", "while away"]));

	events
}

async fn refusing_posts(server: &Server, events: &Events) {
	// Gone connections and unknown ones are refused.
	let missing = Events {
		body: Body::empty(),
		buffer: String::new(),
		connection: "live-0.nope".into(),
	};
	assert_eq!(
		missing.post(server, &[ClientFrame::Ping], &[]).await,
		StatusCode::NOT_FOUND
	);
	assert_eq!(
		events
			.post(
				server,
				&[ClientFrame::Ping],
				&[("origin", "https://elsewhere.example")]
			)
			.await,
		StatusCode::ACCEPTED,
		"without a configured origin, any may post"
	);
}

/// A post must come from the caller the stream belongs to, or start by
/// authenticating, as a socket's `auth` frame does.
async fn posts_belong_to_the_streams_caller(db: Db) {
	let server = serve(db).await;
	let (mut events, _) = Events::open(&server, &[("x-caller", "ada")]).await;
	assert_eq!(
		events
			.post(&server, &[ClientFrame::Ping], &[("x-caller", "ada")])
			.await,
		StatusCode::ACCEPTED
	);
	assert_eq!(events.next().await.1, ServerFrame::Pong);
	assert_eq!(
		events
			.post(&server, &[ClientFrame::Ping], &[("x-caller", "eve")])
			.await,
		StatusCode::FORBIDDEN
	);
	assert_eq!(
		events.post(&server, &[ClientFrame::Ping], &[]).await,
		StatusCode::FORBIDDEN
	);

	// A native client authenticates with a bearer token in a frame.
	let (mut events, _) = Events::open(&server, &[]).await;
	let act = ClientFrame::Act {
		request: 1,
		name: "note_as_caller".into(),
		args: json!({ "body": "hi" }),
	};
	assert_eq!(
		events
			.post(
				&server,
				&[
					ClientFrame::Auth {
						token: "grace".into()
					},
					act.clone()
				],
				&[]
			)
			.await,
		StatusCode::ACCEPTED
	);
	assert!(matches!(
		events.next().await.1,
		ServerFrame::AuthStatus { refused: false, .. }
	));
	let (_, ServerFrame::Done { ok, .. }) = events.next().await else {
		panic!("the action answers");
	};
	assert_eq!(ok, Some(json!("grace")));
	assert_eq!(
		events.post(&server, &[ClientFrame::Ping], &[]).await,
		StatusCode::FORBIDDEN,
		"now it's grace's"
	);
	assert_eq!(
		events
			.post(
				&server,
				&[ClientFrame::Ping],
				&[("authorization", "Bearer grace")]
			)
			.await,
		StatusCode::ACCEPTED
	);
}

/// Draining asks every connection back, over either transport.
async fn draining(db: Db) {
	let server = serve(db).await;
	let (mut socket, _) = connect(&server, None).await;
	let (mut events, _) = Events::open(&server, &[]).await;
	server.live.drain();
	assert!(matches!(
		next(&mut socket).await,
		ServerFrame::Reconnect { .. }
	));
	assert!(matches!(
		events.next().await.1,
		ServerFrame::Reconnect { .. }
	));
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
	connections_share_one_rerun,
	one_log_read_per_commit,
	read_your_writes,
	actions_over_the_connection,
	resuming,
	server_sent_events,
	posts_belong_to_the_streams_caller,
	draining,
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

	connections_share_one_rerun(fresh().await).await;
	one_log_read_per_commit(fresh().await).await;
	read_your_writes(fresh().await).await;
	actions_over_the_connection(fresh().await).await;
	resuming(fresh().await).await;
	server_sent_events(fresh().await).await;
	posts_belong_to_the_streams_caller(fresh().await).await;
	draining(fresh().await).await;
}
