//! The live relay through a real NATS server, on Postgres and SQLite: two
//! servers through it, a mutation on one reaching a subscriber on the
//! other; the stream holding every commit once, in order, chained; the
//! publisher failing over when it crashes; a consumer catching up from the
//! stream after NATS restarts; the database log filling what the stream
//! can't bring (no publisher, a commit too big for a message, a stream that
//! lost messages); channels across servers; and re-runs that never read a
//! replica that's behind.

#![cfg(all(feature = "relay", feature = "postgres", feature = "sqlite"))]

#[path = "support/nats.rs"]
mod nats;
#[path = "../../leptoskit_db/tests/support/postgres.rs"]
mod postgres;

use std::time::Duration;

use async_nats::jetstream;
use axum::Router;
use futures::SinkExt;
use futures::StreamExt;
use leptoskit_db::Db;
use leptoskit_db::Id;
use leptoskit_db::Read;
use leptoskit_db::Statement;
use leptoskit_db::Value as DbValue;
use leptoskit_db::live::LogEntry;
use leptoskit_db::table;
use leptoskit_rpc as rpc;
use leptoskit_rpc::live::ClientFrame;
use leptoskit_rpc::live::ServerFrame;
use leptoskit_rpc::live::Transition;
use leptoskit_rpc::live::Update;
use leptoskit_rpc::server::Live;
use leptoskit_rpc::server::Relay;
use leptoskit_rpc::server::RelaySettings;
use nats::NatsServer;
use postgres::PostgresCluster;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
use serde_json::json;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use tokio_tungstenite::MaybeTlsStream;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;

/// A note everyone may read.
#[table(name = "relay_notes", live)]
pub struct Note {
	pub id: Id<Self>,
	pub body: String,
}

/// Every note's body, in order.
#[rpc::query]
pub async fn relay_titles(cx: &rpc::QueryCx) -> rpc::Result<Vec<String>> {
	let notes = Note::query()
		.order_by(|note| note.body.asc())
		.all(cx.db())
		.await?;
	Ok(notes.into_iter().map(|note| note.body).collect())
}

/// Adds a note.
#[rpc::mutation]
pub async fn add_relay_note(cx: &rpc::MutationCx, body: String) -> rpc::Result<()> {
	NewNote { body }.insert(cx.tx()).await?;
	Ok(())
}

/// Who's in the lobby.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize, schemars::JsonSchema)]
pub struct Here {
	pub name: String,
}

/// A wave across the lobby.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, schemars::JsonSchema)]
pub struct Wave {
	pub from: String,
}

/// A room everyone may join.
#[rpc::channel(message = Wave, presence = Here, authorize = anyone)]
pub struct Lobby {
	pub room: String,
}

#[allow(clippy::unused_async, reason = "the signature the macro expects")]
async fn anyone(cx: &rpc::QueryCx, lobby: &Lobby) -> rpc::Result<()> {
	let _ = (cx, lobby);
	Ok(())
}

fn settings() -> RelaySettings {
	RelaySettings {
		lease: Duration::from_secs(1),
		stall: Duration::from_millis(300),
		..RelaySettings::default()
	}
}

/// Settings under which a consumer never reads the database on its own,
/// so anything it gets beyond the stream came from a gap.
fn patient() -> RelaySettings {
	RelaySettings {
		stall: Duration::from_secs(120),
		..settings()
	}
}

struct Server {
	live: Live,
	url: String,
	task: JoinHandle<()>,
}

impl Drop for Server {
	fn drop(&mut self) {
		self.task.abort();
	}
}

async fn serve(live: Live) -> Server {
	live.start().await.unwrap();
	let app: Router = live.router();
	let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
	let address = listener.local_addr().unwrap();
	let task = tokio::spawn(async move {
		axum::serve(listener, app).await.unwrap();
	});
	Server {
		live,
		url: format!("ws://{address}/live"),
		task,
	}
}

async fn relayed(db: &Db, relay: &Relay) -> Server {
	serve(Live::new(db.clone()).relay(relay)).await
}

type Socket = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

async fn connect(server: &Server) -> Socket {
	let (mut socket, _) = tokio_tungstenite::connect_async(server.url.as_str())
		.await
		.unwrap();
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
		let message = tokio::time::timeout(Duration::from_secs(30), socket.next())
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

async fn subscribe(socket: &mut Socket) -> Vec<String> {
	send(
		socket,
		ClientFrame::Sub {
			id: 1,
			query: "relay_titles".into(),
			args: json!({}),
		},
	)
	.await;

	match next(socket).await {
		ServerFrame::Done {
			ok: Some(value), ..
		} => serde_json::from_value(value).unwrap(),
		other => panic!("expected the first value, got {other:?}"),
	}
}

fn titles(transition: &Transition) -> Option<Vec<String>> {
	transition.updates.iter().find_map(|update| {
		match &update.update {
			Update::Ok(value) => serde_json::from_value(value.clone()).ok(),
			_ => None,
		}
	})
}

/// Reads transitions until one shows `count` notes, checking each comes
/// after the last, and returns the titles.
async fn until_count(socket: &mut Socket, count: usize) -> Vec<String> {
	let mut last = 0;

	loop {
		let ServerFrame::Transition(transition) = next(socket).await else {
			continue;
		};
		assert!(
			transition.to > last || transition.updates.is_empty(),
			"versions only grow"
		);
		last = transition.to.max(last);

		if let Some(titles) = titles(&transition)
			&& titles.len() >= count
		{
			return titles;
		}
	}
}

async fn note(db: &Db, body: &str) {
	NewNote { body: body.into() }.insert(db).await.unwrap();
}

/// Waits until `check` holds, or fails after a while.
async fn eventually(what: &str, mut check: impl FnMut() -> bool) {
	let waited = tokio::time::timeout(Duration::from_secs(30), async {
		while !check() {
			tokio::time::sleep(Duration::from_millis(20)).await;
		}
	})
	.await;
	assert!(waited.is_ok(), "timed out waiting until {what}");
}

async fn fresh_postgres(cluster: &PostgresCluster) -> (String, Db) {
	let url = cluster.database();
	let db = Db::connect(&url).await.unwrap();
	leptoskit_db::testing::create_tables(&db).await.unwrap();
	(url, db)
}

/// Two servers through the relay, each with its own pool: a mutation sent
/// to one reaches a subscriber on the other.
async fn a_mutation_on_one_server_reaches_the_other(first: Db, second: Db, nats: &NatsServer) {
	let relay = Relay::connect_with(&nats.url(), settings()).await.unwrap();
	let publisher = relay.publish(&first);
	eventually("one server publishes", || publisher.stats().following).await;
	let a = relayed(&first, &relay).await;
	let other_relay = Relay::connect_with(&nats.url(), settings()).await.unwrap();
	let b = relayed(&second, &other_relay).await;

	let mut watcher = connect(&b).await;
	assert_eq!(subscribe(&mut watcher).await, Vec::<String>::new());
	let mut writer = connect(&a).await;
	send(
		&mut writer,
		ClientFrame::Mutate {
			request: 9,
			name: "add_relay_note".into(),
			args: json!({ "body": "across" }),
			key: None,
		},
	)
	.await;
	assert!(matches!(
		next(&mut writer).await,
		ServerFrame::Done {
			request: 9,
			err: None,
			..
		}
	));

	assert_eq!(until_count(&mut watcher, 1).await, ["across"]);
	assert!(b.live.stats().commits >= 1);
	assert!(publisher.stats().commits >= 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn two_servers_on_sqlite() {
	let Some(nats) = NatsServer::start() else {
		return;
	};
	// One file, two pools: two servers sharing a database.
	let directory = tempfile::tempdir().unwrap();
	let url = format!(
		"sqlite://{}",
		directory.path().join("app.sqlite3").display()
	);
	let first = Db::connect(&url).await.unwrap();
	leptoskit_db::testing::create_tables(&first).await.unwrap();
	let second = Db::connect(&url).await.unwrap();
	a_mutation_on_one_server_reaches_the_other(first, second, &nats).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn two_servers_on_postgres() {
	let (Some(cluster), Some(nats)) = (PostgresCluster::start(), NatsServer::start()) else {
		return;
	};
	let (url, first) = fresh_postgres(&cluster).await;
	let second = Db::connect(&url).await.unwrap();
	a_mutation_on_one_server_reaches_the_other(first, second, &nats).await;
}

/// Every message on the stream, from the first.
async fn stream_batches(nats: &NatsServer, stream: &str) -> Vec<(i64, i64, Vec<LogEntry>)> {
	let client = async_nats::connect(nats.url()).await.unwrap();
	let js = jetstream::new(client);
	let mut stream = js.get_stream(stream).await.unwrap();
	let last = stream.info().await.unwrap().state.last_sequence;
	let mut batches = Vec::new();

	for sequence in 1..=last {
		let Ok(message) = stream.get_raw_message(sequence).await else {
			continue;
		};
		let batch: Value = serde_json::from_slice(&message.payload).unwrap();
		let entries: Vec<LogEntry> =
			serde_json::from_value(batch["entries"].clone()).unwrap_or_default();
		batches.push((
			batch["from"].as_i64().unwrap(),
			batch["to"].as_i64().unwrap(),
			entries,
		));
	}

	batches
}

/// Checks the stream chains from message to message and holds each commit
/// once, in order; returns the versions it holds.
fn chained(batches: &[(i64, i64, Vec<LogEntry>)]) -> Vec<i64> {
	let mut versions = Vec::new();

	for pair in batches.windows(2) {
		assert_eq!(
			pair[1].0, pair[0].1,
			"each message starts where the last ended"
		);
	}

	for (from, to, entries) in batches {
		let mut last = *from;

		for entry in entries {
			let version = match entry {
				LogEntry::Commit(commit) => commit.version,
				LogEntry::Pruned { through } => *through,
			};
			assert!(version > last, "in order, once");
			last = version;
			versions.push(version);
		}

		assert_eq!(last, *to);
	}

	versions
}

async fn committed_versions(db: &Db, after: i64) -> Vec<i64> {
	db.fetch(Statement {
		sql: format!(
			"SELECT version FROM live.commits WHERE version > {after} AND origin <> \
			 'leptoskit.pruned' ORDER BY version"
		),
		args: Vec::new(),
	})
	.await
	.unwrap()
	.into_iter()
	.map(|row| {
		match row[0] {
			DbValue::Int(version) => version,
			ref other => panic!("{other:?}"),
		}
	})
	.collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn concurrent_writers_reach_the_stream_once_each_in_order() {
	let (Some(cluster), Some(nats)) = (PostgresCluster::start(), NatsServer::start()) else {
		return;
	};
	let (_url, db) = fresh_postgres(&cluster).await;
	let start = db.live_version().await.unwrap();
	let relay = Relay::connect_with(&nats.url(), patient()).await.unwrap();
	let publisher = relay.publish(&db);
	eventually("it follows the log", || publisher.stats().following).await;
	let server = relayed(&db, &relay).await;
	let mut watcher = connect(&server).await;
	subscribe(&mut watcher).await;

	let writers: Vec<_> = (0..8)
		.map(|writer| {
			let db = db.clone();
			tokio::spawn(async move {
				for count in 0..25 {
					note(&db, &format!("{writer}-{count:02}")).await;
				}
			})
		})
		.collect();

	for writer in writers {
		writer.await.unwrap();
	}

	assert_eq!(until_count(&mut watcher, 200).await.len(), 200);
	assert_eq!(
		server.live.stats().relay_fills,
		0,
		"everything came through the stream"
	);

	let batches = stream_batches(
		&nats,
		&format!("LEPTOSKIT_LIVE_{}", database_name(&db).await),
	)
	.await;
	let versions = chained(&batches);
	assert_eq!(versions, committed_versions(&db, start).await);
}

async fn database_name(db: &Db) -> String {
	match &db
		.fetch(Statement {
			sql: "SELECT current_database()".into(),
			args: Vec::new(),
		})
		.await
		.unwrap()[0][0]
	{
		DbValue::Text(name) => name.clone(),
		other => panic!("{other:?}"),
	}
}

#[tokio::test(flavor = "multi_thread")]
async fn the_publisher_fails_over_when_it_crashes() {
	let (Some(cluster), Some(nats)) = (PostgresCluster::start(), NatsServer::start()) else {
		return;
	};
	let (url, db) = fresh_postgres(&cluster).await;
	let start = db.live_version().await.unwrap();

	// The first publisher runs on a runtime of its own, so dropping the
	// runtime kills it mid-flight, without giving up the lease.
	let (crash, crashed) = std::sync::mpsc::channel::<()>();
	let (leading, led) = tokio::sync::oneshot::channel();
	let nats_url = nats.url();
	let first = std::thread::spawn(move || {
		let runtime = tokio::runtime::Builder::new_multi_thread()
			.enable_all()
			.build()
			.unwrap();
		runtime.block_on(async {
			let db = Db::connect(&url).await.unwrap();
			let relay = Relay::connect_with(&nats_url, settings()).await.unwrap();
			let publisher = relay.publish(&db);
			while !publisher.stats().following {
				tokio::time::sleep(Duration::from_millis(10)).await;
			}
			leading.send(()).unwrap();
			std::mem::forget(publisher);
		});
		crashed.recv().unwrap();
		runtime.shutdown_background();
	});
	led.await.unwrap();

	let relay = Relay::connect_with(&nats.url(), patient()).await.unwrap();
	let second = relay.publish(&db);
	let server = relayed(&db, &relay).await;
	let mut watcher = connect(&server).await;
	subscribe(&mut watcher).await;

	for count in 0..5 {
		note(&db, &format!("before-{count}")).await;
	}
	assert_eq!(until_count(&mut watcher, 5).await.len(), 5);
	assert!(!second.stats().leading, "one publisher at a time");

	crash.send(()).unwrap();
	first.join().unwrap();

	for count in 0..5 {
		note(&db, &format!("after-{count}")).await;
	}
	assert_eq!(until_count(&mut watcher, 10).await.len(), 10);
	assert!(second.stats().leading, "the second took over");
	assert!(second.stats().commits >= 1);
	assert_eq!(server.live.stats().relay_fills, 0, "the stream had it all");

	let batches = stream_batches(
		&nats,
		&format!("LEPTOSKIT_LIVE_{}", database_name(&db).await),
	)
	.await;
	assert_eq!(chained(&batches), committed_versions(&db, start).await);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_consumer_catches_up_from_the_stream_after_nats_restarts() {
	let (Some(cluster), Some(mut nats)) = (PostgresCluster::start(), NatsServer::start()) else {
		return;
	};
	let (_url, db) = fresh_postgres(&cluster).await;
	let relay = Relay::connect_with(&nats.url(), patient()).await.unwrap();
	let publisher = relay.publish(&db);
	eventually("it follows the log", || publisher.stats().following).await;
	let server = relayed(&db, &relay).await;
	let mut watcher = connect(&server).await;
	subscribe(&mut watcher).await;
	note(&db, "first").await;
	assert_eq!(until_count(&mut watcher, 1).await, ["first"]);

	nats.stop();
	note(&db, "second").await;
	note(&db, "third").await;
	tokio::time::sleep(Duration::from_millis(300)).await;
	nats.restart();

	assert_eq!(
		until_count(&mut watcher, 3).await,
		["first", "second", "third"]
	);
	assert_eq!(
		server.live.stats().relay_fills,
		0,
		"the publisher published the backlog, and the consumer read it from the stream"
	);
}

#[tokio::test(flavor = "multi_thread")]
async fn without_a_publisher_consumers_read_the_database() {
	let (Some(cluster), Some(nats)) = (PostgresCluster::start(), NatsServer::start()) else {
		return;
	};
	let (_url, db) = fresh_postgres(&cluster).await;
	let relay = Relay::connect_with(&nats.url(), settings()).await.unwrap();
	let server = relayed(&db, &relay).await;
	let mut watcher = connect(&server).await;
	subscribe(&mut watcher).await;
	note(&db, "unpublished").await;
	assert_eq!(until_count(&mut watcher, 1).await, ["unpublished"]);
	assert!(server.live.stats().relay_fills >= 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_commit_too_big_for_a_message_is_read_from_the_database() {
	let (Some(cluster), Some(nats)) = (PostgresCluster::start(), NatsServer::start()) else {
		return;
	};
	let (_url, db) = fresh_postgres(&cluster).await;
	let relay = Relay::connect_with(
		&nats.url(),
		RelaySettings {
			max_message_bytes: 1_500,
			..patient()
		},
	)
	.await
	.unwrap();
	let publisher = relay.publish(&db);
	eventually("it follows the log", || publisher.stats().following).await;
	let server = relayed(&db, &relay).await;
	let mut watcher = connect(&server).await;
	subscribe(&mut watcher).await;

	note(&db, "small").await;
	assert_eq!(until_count(&mut watcher, 1).await, ["small"]);
	assert_eq!(server.live.stats().relay_fills, 0);

	let big = "b".repeat(5_000);
	note(&db, &big).await;
	assert_eq!(
		until_count(&mut watcher, 2).await,
		[big, "small".to_owned()]
	);
	assert_eq!(server.live.stats().relay_fills, 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_gap_in_the_stream_is_read_from_the_database() {
	let (Some(cluster), Some(nats)) = (PostgresCluster::start(), NatsServer::start()) else {
		return;
	};
	let (_url, db) = fresh_postgres(&cluster).await;
	let relay = Relay::connect_with(&nats.url(), patient()).await.unwrap();
	let publisher = relay.publish(&db);
	eventually("it follows the log", || publisher.stats().following).await;
	let server = relayed(&db, &relay).await;
	let mut watcher = connect(&server).await;
	subscribe(&mut watcher).await;
	note(&db, "one").await;
	assert_eq!(until_count(&mut watcher, 1).await, ["one"]);

	// The publisher stops, two commits go unpublished, and the stream loses
	// what it had, as retention would: the next publisher starts after them.
	publisher.shutdown().await;
	note(&db, "two").await;
	note(&db, "three").await;
	let client = async_nats::connect(nats.url()).await.unwrap();
	jetstream::new(client)
		.get_stream(format!("LEPTOSKIT_LIVE_{}", database_name(&db).await))
		.await
		.unwrap()
		.purge()
		.await
		.unwrap();
	let publisher = relay.publish(&db);
	eventually("it follows the log again", || publisher.stats().following).await;
	note(&db, "four").await;

	assert_eq!(
		until_count(&mut watcher, 4).await,
		["four", "one", "three", "two"]
	);
	assert_eq!(
		server.live.stats().relay_fills,
		2,
		"the two the stream never had"
	);
}

async fn join(socket: &mut Socket, name: &str) {
	send(
		socket,
		ClientFrame::Join {
			channel: "lobby-1".into(),
			name: "lobby".into(),
			args: json!({ "room": "1" }),
			state: json!({ "name": name }),
		},
	)
	.await;
}

/// Reads frames until a presence frame shows `count` members, and returns
/// their names.
async fn presence(socket: &mut Socket, count: usize) -> Vec<String> {
	loop {
		if let ServerFrame::Presence { members, .. } = next(socket).await
			&& members.len() == count
		{
			let mut names: Vec<String> = members
				.into_iter()
				.map(|member| member.state["name"].as_str().unwrap().to_owned())
				.collect();
			names.sort();
			return names;
		}
	}
}

#[tokio::test(flavor = "multi_thread")]
async fn channels_cross_servers() {
	let (Some(cluster), Some(nats)) = (PostgresCluster::start(), NatsServer::start()) else {
		return;
	};
	let (url, first) = fresh_postgres(&cluster).await;
	let second = Db::connect(&url).await.unwrap();
	let relay = Relay::connect_with(&nats.url(), settings()).await.unwrap();
	let other_relay = Relay::connect_with(&nats.url(), settings()).await.unwrap();
	let a = relayed(&first, &relay).await;
	let b = relayed(&second, &other_relay).await;

	let mut ada = connect(&a).await;
	let mut grace = connect(&b).await;
	join(&mut ada, "ada").await;
	assert_eq!(presence(&mut ada, 1).await, ["ada"]);
	join(&mut grace, "grace").await;
	assert_eq!(presence(&mut grace, 2).await, ["ada", "grace"]);
	assert_eq!(presence(&mut ada, 2).await, ["ada", "grace"]);

	send(
		&mut ada,
		ClientFrame::Signal {
			channel: "lobby-1".into(),
			message: json!({ "from": "ada" }),
		},
	)
	.await;
	loop {
		if let ServerFrame::Signal { message, .. } = next(&mut grace).await {
			assert_eq!(message, json!({ "from": "ada" }));
			break;
		}
	}

	drop(grace);
	assert_eq!(presence(&mut ada, 1).await, ["ada"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn reruns_never_read_a_replica_that_is_behind() {
	let Some(cluster) = PostgresCluster::start() else {
		return;
	};
	let standby = cluster.standby();
	let (url, db) = fresh_postgres(&cluster).await;
	let db = db.with_replica(&standby.url(&url)).unwrap();
	let server = serve(Live::new(db.clone()).replica_wait(Duration::from_millis(100))).await;
	let mut watcher = connect(&server).await;
	subscribe(&mut watcher).await;

	// Streaming keeps the standby a moment behind, so re-runs read it once
	// it has replayed each commit.
	let mut count = 0;
	while server.live.stats().replica_reruns == 0 {
		count += 1;
		note(&db, &format!("streamed-{count:03}")).await;
		assert_eq!(until_count(&mut watcher, count).await.len(), count);
		assert!(count < 100, "re-runs never read the standby");
	}

	// Paused, the standby never shows the next note, so a re-run that read
	// it would leave the note out.
	standby.pause_replay();
	let before = server.live.stats().replica_reruns;

	for paused in 0..3 {
		count += 1;
		let body = format!("paused-{paused}");
		note(&db, &body).await;
		let titles = until_count(&mut watcher, count).await;
		assert!(titles.contains(&body), "the re-run read the primary");
	}

	assert_eq!(server.live.stats().replica_reruns, before);

	standby.resume_replay();
	let resumed = server.live.stats().replica_reruns;

	while server.live.stats().replica_reruns == resumed {
		count += 1;
		note(&db, &format!("resumed-{count:03}")).await;
		assert_eq!(until_count(&mut watcher, count).await.len(), count);
		assert!(count < 200, "re-runs never read the standby again");
	}
}
