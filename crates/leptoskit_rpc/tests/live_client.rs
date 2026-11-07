//! The live client against a real server in this process: subscriptions and
//! unsubscribing, reconnecting after a drain, a resumable stream reopened
//! in place, credits, a dropped stream, channel refusals, and the globals a
//! build with no connection has.

#![cfg(all(feature = "live", feature = "sqlite"))]

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::Once;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::time::Duration;

use futures::Stream;
use futures::StreamExt;
use leptos::prelude::Get as _;
use leptos::prelude::Owner;
use leptoskit_db::Db;
use leptoskit_db::Id;
use leptoskit_db::table;
use leptoskit_query::BoxFuture;
use leptoskit_query::QueryClient;
use leptoskit_rpc as rpc;
use leptoskit_rpc::FnError;
use leptoskit_rpc::client::ChannelEvent;
use leptoskit_rpc::client::ClientConfig;
use leptoskit_rpc::client::Credentials;
use leptoskit_rpc::client::LiveState;
use leptoskit_rpc::client::StreamEvent;
use leptoskit_rpc::live::Update;
use leptoskit_rpc::server::Live;
use leptoskit_rpc::testing::LiveClients;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
use serde_json::json;
use tokio::task::LocalSet;

/// A note everyone may read.
#[table(name = "client_notes", live)]
pub struct Note {
	pub id: Id<Self>,
	pub body: String,
}

/// Every note's body, in order.
#[rpc::query]
pub async fn client_bodies(cx: &rpc::QueryCx) -> rpc::Result<Vec<String>> {
	let notes = Note::query()
		.order_by(|note| note.body.asc())
		.all(cx.db())
		.await?;
	Ok(notes.into_iter().map(|note| note.body).collect())
}

/// A caller's own note, which only `ada` may read.
#[rpc::query]
pub async fn client_private(cx: &rpc::QueryCx) -> rpc::Result<Vec<String>> {
	if cx.identity().caller.as_deref() != Some("ada") {
		return Err(rpc::Error::forbidden("not yours"));
	}

	Ok(vec!["ada's".to_owned()])
}

/// Writes a note.
#[rpc::mutation]
pub async fn client_add(cx: &rpc::MutationCx, body: String) -> rpc::Result<()> {
	NewNote { body }.insert(cx.tx()).await?;
	Ok(())
}

/// Where a resumable stream reached.
#[derive(Clone, Deserialize, PartialEq, Serialize)]
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
pub async fn client_ticks(
	_cx: &rpc::QueryCx,
	cursor: Option<u32>,
) -> rpc::Result<impl Stream<Item = Tick>> {
	let start = cursor.map_or(0, |cursor| cursor + 1);
	Ok(futures::stream::unfold(start, |n| {
		async move {
			tokio::time::sleep(Duration::from_millis(5)).await;
			Some((Tick { n }, n + 1))
		}
	}))
}

/// The numbers below `upto`.
#[rpc::stream]
pub async fn client_count(cx: &rpc::QueryCx, upto: u32) -> rpc::Result<impl Stream<Item = u32>> {
	let _ = cx;
	Ok(futures::stream::iter(0..upto))
}

/// Whether a dropped stream's body went away.
static DROPPED: AtomicBool = AtomicBool::new(false);

/// Marks its run dropped when it goes.
struct Guard;

impl Drop for Guard {
	fn drop(&mut self) {
		DROPPED.store(true, Ordering::SeqCst);
	}
}

/// Ticks forever, and says if its body was dropped.
#[rpc::stream]
pub async fn client_held(cx: &rpc::QueryCx) -> rpc::Result<impl Stream<Item = u32>> {
	let _ = cx;
	Ok(futures::stream::unfold((0, Guard), |(n, guard)| {
		async move {
			tokio::time::sleep(Duration::from_millis(5)).await;
			Some((n, (n + 1, guard)))
		}
	}))
}

/// What members send each other.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Wave {
	pub from: String,
}

/// What a member shows the others.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct Seat {
	pub name: String,
}

/// A room, refused for `secret`.
#[rpc::channel(message = Wave, presence = Seat, authorize = may_join)]
pub struct ClientRoom {
	pub room: String,
}

#[allow(clippy::unused_async, reason = "the signature the macro expects")]
async fn may_join(_cx: &rpc::QueryCx, room: &ClientRoom) -> rpc::Result<()> {
	if room.room == "secret" {
		return Err(rpc::Error::forbidden("that room is private"));
	}

	Ok(())
}

/// Tells callers apart by an `x-caller` header or a bearer token.
struct Caller;

impl rpc::server::Identify for Caller {
	fn identify<'a>(
		&'a self,
		request: &'a http::request::Parts,
	) -> std::pin::Pin<Box<dyn Future<Output = rpc::Result<rpc::server::Identity>> + Send + 'a>>
	{
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

			Ok(
				caller.map_or_else(rpc::server::Identity::anonymous, |caller| {
					rpc::server::Identity::anonymous().with_caller(caller)
				}),
			)
		})
	}
}

/// A bearer token the test can change.
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

static START: Once = Once::new();

fn start() {
	START.call_once(|| {
		any_spawner::Executor::init_tokio().unwrap();
	});
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
	let mut value = Value::Null;

	loop {
		let update = tokio::time::timeout(Duration::from_secs(20), updates.next())
			.await
			.expect("an update arrives")
			.expect("the subscription stays");
		rpc::live::apply(&mut value, &update);

		if matches!(update, Update::Ok(_)) {
			return value;
		}
	}
}

async fn next_tick(ticks: &mut rpc::client::LiveStream) -> u32 {
	match tokio::time::timeout(Duration::from_secs(20), ticks.next())
		.await
		.expect("a tick arrives")
	{
		Some(StreamEvent::Item(value)) => serde_json::from_value::<Tick>(value).unwrap().n,
		other => panic!("expected a tick, got {other:?}"),
	}
}

fn live(db: Db) -> Live {
	Live::new(db).identify(Caller)
}

/// A build with no installed client reports no connection, and the globals
/// do nothing.
#[test]
fn without_an_installed_client_the_globals_are_offline() {
	rpc::client::configure(ClientConfig::new());
	assert!(!rpc::client::live::available());
	rpc::client::live::reauthenticate();
	// Signal hooks run inside a component owner, including builds using
	// sandboxed arenas. Nextest runs this test without another test having
	// initialized an arena on its thread.
	Owner::new().with(|| {
		let status = rpc::client::live::use_live_status();
		assert_eq!(status.get(), rpc::client::LiveStatus::OFFLINE);
	});
}

/// A subscription takes its value, stops when unsubscribed, and status
/// watchers hear where the connection is.
#[tokio::test(flavor = "multi_thread")]
async fn subscriptions_stop_when_unsubscribed() {
	start();
	let db = leptoskit_db::testing::sqlite().await;
	let live = live(db.clone());
	let clients = LiveClients::new(&live);

	LocalSet::new()
		.run_until(async {
			let client = clients.client(&ClientConfig::new());
			wait_for("the connection", || client.status().is_live()).await;
			assert!(client.carries_calls());
			let (id, mut updates) = client.subscribe("client_bodies", json!({}));
			assert_eq!(next_update(&mut updates).await, json!([]));
			let mut statuses = client.watch_status();

			NewNote {
				body: "first".into(),
			}
			.insert(&db)
			.await
			.unwrap();
			assert_eq!(next_update(&mut updates).await, json!(["first"]));
			assert!(client.version() >= 1);
			wait_for("the status", || {
				statuses
					.try_recv()
					.is_ok_and(|status| matches!(status.state, LiveState::Live { .. }))
			})
			.await;

			client.unsubscribe(id);
			assert!(
				tokio::time::timeout(Duration::from_millis(300), updates.next())
					.await
					.unwrap()
					.is_none(),
				"the subscription ended"
			);
			NewNote {
				body: "after".into(),
			}
			.insert(&db)
			.await
			.unwrap();
			tokio::time::sleep(Duration::from_millis(100)).await;
			assert_eq!(client.status().queued, 0);
		})
		.await;
}

/// A drain reconnects the client, resumes what it holds, and keeps it live.
#[tokio::test(flavor = "multi_thread")]
async fn a_drain_reconnects_and_resumes() {
	start();
	let db = leptoskit_db::testing::sqlite().await;
	let live = live(db.clone());
	let clients = LiveClients::new(&live);

	LocalSet::new()
		.run_until(async {
			let client = clients.client(&ClientConfig::new());
			wait_for("the connection", || client.status().is_live()).await;
			let (_, mut updates) = client.subscribe("client_bodies", json!({}));
			assert_eq!(next_update(&mut updates).await, json!([]));
			let reached = client.version();

			live.drain();
			wait_for("the reconnect", || client.status().is_live()).await;
			assert!(client.version() >= reached);
			NewNote {
				body: "after the drain".into(),
			}
			.insert(&db)
			.await
			.unwrap();
			assert_eq!(next_update(&mut updates).await, json!(["after the drain"]));
		})
		.await;
}

/// A resumable stream is interrupted in place when the identity is checked
/// again, and opens from its cursor.
#[tokio::test(flavor = "multi_thread")]
async fn a_resumable_stream_reopens_in_place() {
	start();
	let db = leptoskit_db::testing::sqlite().await;
	let live = live(db);
	let clients = LiveClients::new(&live);
	clients.header("x-caller", Some("ada"));

	LocalSet::new()
		.run_until(async {
			let token = Token::default();
			token.set(Some("ada"));
			let client = clients.client(&ClientConfig::new().credentials(token));
			wait_for("the connection", || client.status().is_live()).await;
			let mut ticks = client.stream("client_ticks", json!({}), 8, true);
			assert_ne!(ticks.id(), 0);
			let mut seen = Vec::new();

			while seen.len() < 3 {
				seen.push(next_tick(&mut ticks).await);
			}

			client.reauthenticate();

			while seen.len() < 6 {
				seen.push(next_tick(&mut ticks).await);
			}

			assert_eq!(seen, (0..6).collect::<Vec<_>>(), "no gap and no repeat");
		})
		.await;
}

/// Taking a stream's items grants the server more room, and dropping a
/// stream cancels it.
#[tokio::test(flavor = "multi_thread")]
async fn stream_credits_and_cancellation() {
	start();
	let db = leptoskit_db::testing::sqlite().await;
	let live = live(db);
	let clients = LiveClients::new(&live);

	LocalSet::new()
		.run_until(async {
			let client = clients.client(&ClientConfig::new());
			wait_for("the connection", || client.status().is_live()).await;
			let mut count = client.stream("client_count", json!({ "upto": 20 }), 2, false);
			let mut seen = Vec::new();

			while seen.len() < 5 {
				match tokio::time::timeout(Duration::from_secs(20), count.next())
					.await
					.expect("an item arrives")
				{
					Some(StreamEvent::Item(value)) => seen.push(value),
					other => panic!("expected an item, got {other:?}"),
				}
			}

			assert_eq!(seen, [json!(0), json!(1), json!(2), json!(3), json!(4)]);
			drop(count);

			DROPPED.store(false, Ordering::SeqCst);
			let held = client.stream("client_held", json!({}), 2, false);
			tokio::time::sleep(Duration::from_millis(100)).await;
			drop(held);
			wait_for("the stream to be dropped", || {
				DROPPED.load(Ordering::SeqCst)
			})
			.await;
		})
		.await;
}

/// A refused join reaches the app as an error, and a joined channel carries
/// presence and signals.
#[tokio::test(flavor = "multi_thread")]
async fn channel_errors_and_events_reach_the_app() {
	start();
	let db = leptoskit_db::testing::sqlite().await;
	let live = live(db);
	let clients = LiveClients::new(&live);

	LocalSet::new()
		.run_until(async {
			let client = clients.client(&ClientConfig::new());
			let other = clients.client(&ClientConfig::new());
			wait_for("the connections", || {
				client.status().is_live() && other.status().is_live()
			})
			.await;

			let (key, mut events) = client.join(
				ClientRoom {
					room: "secret".into(),
				},
				None,
			);
			assert!(matches!(
				tokio::time::timeout(Duration::from_secs(20), events.next())
					.await
					.expect("a refusal")
					.expect("the channel stays"),
				ChannelEvent::Error(_)
			));
			client.leave(&key);

			let (key, mut events) = client.join(
				ClientRoom {
					room: "lobby".into(),
				},
				Some(Seat { name: "ada".into() }),
			);
			assert!(matches!(
				tokio::time::timeout(Duration::from_secs(20), events.next())
					.await
					.expect("the join's presence"),
				Some(ChannelEvent::Presence(_))
			));
			let (other_key, mut other_events) = other.join(
				ClientRoom {
					room: "lobby".into(),
				},
				Some(Seat {
					name: "grace".into(),
				}),
			);
			let presence = loop {
				if let Some(ChannelEvent::Presence(members)) =
					tokio::time::timeout(Duration::from_secs(20), other_events.next())
						.await
						.expect("presence")
				{
					break members;
				}
			};
			assert_eq!(presence.len(), 2, "{presence:?}");

			other.signal::<ClientRoom>(
				&other_key,
				&Wave {
					from: "grace".into(),
				},
			);
			let signal = loop {
				match tokio::time::timeout(Duration::from_secs(20), events.next())
					.await
					.expect("a signal")
				{
					Some(ChannelEvent::Signal { message, .. }) => break message,
					Some(_) => {}
					None => panic!("the channel closed"),
				}
			};
			assert_eq!(signal, json!({ "from": "grace" }));
			client.leave(&key);
		})
		.await;
}

/// Commands for a subscription, channel, or stream that went away before
/// they ran are skipped rather than sent.
#[tokio::test(flavor = "multi_thread")]
async fn commands_for_removed_state_are_skipped() {
	start();
	let db = leptoskit_db::testing::sqlite().await;
	let live = live(db.clone());
	let clients = LiveClients::new(&live);

	LocalSet::new()
		.run_until(async {
			let client = clients.client(&ClientConfig::new());
			wait_for("the connection", || client.status().is_live()).await;

			let (id, _updates) = client.subscribe("client_bodies", json!({}));
			client.unsubscribe(id);
			let (key, _events) = client.join(
				ClientRoom {
					room: "lobby".into(),
				},
				None,
			);
			client.leave(&key);
			let held = client.stream("client_held", json!({}), 2, false);
			drop(held);

			tokio::time::sleep(Duration::from_millis(100)).await;
			assert!(client.status().is_live());
			NewNote {
				body: "still here".into(),
			}
			.insert(&db)
			.await
			.unwrap();
			tokio::time::sleep(Duration::from_millis(100)).await;
			assert!(client.status().is_live());
		})
		.await;
}

/// The app's installed client can be asked to read as its credentials now
/// say.
#[tokio::test(flavor = "multi_thread")]
async fn the_global_client_reauthenticates() {
	start();
	let db = leptoskit_db::testing::sqlite().await;
	let live = live(db);
	let clients = LiveClients::new(&live);

	LocalSet::new()
		.run_until(async {
			assert!(
				clients.install(&ClientConfig::new()),
				"one client a process"
			);
			let client = rpc::client::live::client();
			wait_for("the connection", || client.status().is_live()).await;
			rpc::client::live::reauthenticate();
			tokio::time::sleep(Duration::from_millis(100)).await;
			assert!(client.status().is_live());
		})
		.await;
}

/// Without a live connection the hooks are ordinary calls, in process.
#[tokio::test(flavor = "multi_thread")]
async fn live_hooks_without_a_connection_use_the_ordinary_client() {
	start();
	let db = leptoskit_db::testing::sqlite().await;

	LocalSet::new()
		.run_until(async {
			let owner = Owner::new();
			owner.set();
			QueryClient::new().provide();
			rpc::server::Server::new(db.clone()).install();

			let bodies = rpc::use_live(|| ClientBodies {});
			wait_for("the rows", || bodies.data().is_some()).await;
			let send = rpc::use_live_mutation::<ClientAdd>();
			send.mutate_async(ClientAdd {
				body: "offline".into(),
			})
			.await
			.unwrap();
			let notes = Note::query().all(&db).await.unwrap();
			assert_eq!(notes.len(), 1);
			assert_eq!(notes[0].body, "offline");
		})
		.await;
}

/// A transition's error reaches its subscriber.
#[tokio::test(flavor = "multi_thread")]
async fn a_transition_error_reaches_its_subscriber() {
	start();
	let db = leptoskit_db::testing::sqlite().await;
	let live = live(db);
	let clients = LiveClients::new(&live);
	clients.header("x-caller", Some("ada"));

	LocalSet::new()
		.run_until(async {
			let token = Token::default();
			token.set(Some("ada"));
			let client = clients.client(&ClientConfig::new().credentials(token.clone()));
			wait_for("the connection", || client.status().is_live()).await;
			let (_, mut updates) = client.subscribe("client_private", json!({}));
			assert_eq!(next_update(&mut updates).await, json!(["ada's"]));

			token.set(Some("eve"));
			client.reauthenticate();
			let error = loop {
				match tokio::time::timeout(Duration::from_secs(20), updates.next())
					.await
					.expect("an update arrives")
				{
					Some(Update::Err(error)) => break error,
					Some(Update::Ok(value)) => assert_eq!(value, json!(["ada's"])),
					Some(Update::Patch(_)) => {}
					None => panic!("the subscription ended"),
				}
			};
			assert_eq!(error.status, 403, "{error:?}");
		})
		.await;
}
