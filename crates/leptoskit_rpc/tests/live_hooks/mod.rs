//! The client hooks against a real server in this process: the default
//! mutation hook's read-your-writes over the live connection, pinned pages,
//! and a stream on screen. A process holds one app client, so SQLite and
//! Postgres each run in their own test binary.

use std::time::Duration;

use any_spawner::Executor;
use futures::Stream;
use leptos::prelude::*;
use leptoskit_db::Db;
use leptoskit_db::Id;
use leptoskit_db::table;
use leptoskit_query::QueryClient;
use leptoskit_query::use_mutation;
use leptoskit_rpc as rpc;
use leptoskit_rpc::client::ClientConfig;
use leptoskit_rpc::client::StreamState;
use leptoskit_rpc::client::mutation_scope;
use leptoskit_rpc::server::Live;
use leptoskit_rpc::testing::LiveClients;
use tokio::task::LocalSet;

/// A message, in the order it was written.
#[table(name = "hook_messages", live)]
pub struct Message {
	pub id: Id<Self>,
	pub body: String,
}

/// A message as a page shows it.
#[derive(
	Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
pub struct Row {
	pub id: String,
	pub body: String,
}

/// Every message's body, oldest first.
#[rpc::query]
pub async fn hook_bodies(cx: &rpc::QueryCx) -> rpc::Result<Vec<String>> {
	let rows = Message::query()
		.order_by(|message| message.id.asc())
		.all(cx.db())
		.await?;
	Ok(rows.into_iter().map(|message| message.body).collect())
}

/// The messages, newest first, a page at a time.
#[rpc::query]
pub async fn hook_history(cx: &rpc::QueryCx, page: rpc::PageArgs) -> rpc::Result<rpc::Page<Row>> {
	let page = rpc::paginate(
		cx.db(),
		Message::query(),
		&page,
		|message| message.id.desc(),
		|message| message.id,
	)
	.await?;
	Ok(rpc::Page {
		items: page
			.items
			.into_iter()
			.map(|message| {
				Row {
					id: message.id.to_string(),
					body: message.body,
				}
			})
			.collect(),
		end: page.end,
		has_more: page.has_more,
	})
}

/// Every message as a row, oldest first: a list the hub patches.
#[rpc::query]
pub async fn hook_rows(cx: &rpc::QueryCx) -> rpc::Result<Vec<Row>> {
	let messages = Message::query()
		.order_by(|message| message.id.asc())
		.all(cx.db())
		.await?;
	Ok(messages
		.into_iter()
		.map(|message| {
			Row {
				id: message.id.to_string(),
				body: message.body,
			}
		})
		.collect())
}

/// Writes a message.
#[rpc::mutation]
pub async fn hook_write(cx: &rpc::MutationCx, body: String) -> rpc::Result<()> {
	NewMessage { body }.insert(cx.tx()).await?;
	Ok(())
}

/// The numbers below `upto`.
#[rpc::stream]
pub async fn hook_count(cx: &rpc::QueryCx, upto: u32) -> rpc::Result<impl Stream<Item = u32>> {
	let _ = cx;
	Ok(futures::stream::iter(0..upto))
}

/// One number, then a failure.
#[rpc::stream]
pub async fn hook_broken(cx: &rpc::QueryCx) -> rpc::Result<impl Stream<Item = rpc::Result<u32>>> {
	let _ = cx;
	Ok(futures::stream::iter([
		Ok(1),
		Err(rpc::Error::conflict("the stream broke")),
	]))
}

/// What members of a hook room send each other.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Wave {
	pub from: String,
}

/// What a member of a hook room shows the others.
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct Seat {
	pub name: String,
}

/// A room anyone may join.
#[rpc::channel(message = Wave, presence = Seat, authorize = may_sit)]
pub struct HookRoom {
	pub room: String,
}

/// The same room, but its messages are raw JSON, so a test can send one
/// that doesn't fit what the screen expects.
#[derive(Clone, serde::Deserialize, serde::Serialize)]
pub struct HookRoomAnything {
	pub room: String,
}

impl rpc::Channel for HookRoomAnything {
	type Message = serde_json::Value;
	type Presence = Seat;

	const NAME: &'static str = "hook_room";
}

#[allow(clippy::unused_async, reason = "the signature the macro expects")]
async fn may_sit(cx: &rpc::QueryCx, room: &HookRoom) -> rpc::Result<()> {
	let _ = cx;

	if room.room == "secret" {
		return Err(rpc::Error::forbidden("that room is private"));
	}

	Ok(())
}

/// Shouts `body` back.
#[rpc::action]
pub async fn hook_shout(cx: &rpc::ActionCx, body: String) -> rpc::Result<String> {
	let _ = cx;
	Ok(body.to_uppercase())
}

/// Waits for `read` to return `Some`.
async fn eventually<T>(what: &str, read: impl Fn() -> Option<T>) -> T {
	for _ in 0..1_000 {
		if let Some(value) = read() {
			return value;
		}

		tokio::time::sleep(Duration::from_millis(10)).await;
	}

	panic!("waited too long for {what}");
}

async fn write(db: &Db, body: &str) {
	NewMessage { body: body.into() }.insert(db).await.unwrap();
}

/// Runs everything on the test's `LocalSet`, as a browser runs everything
/// on its one thread: an effect that spawns a local task needs to be on it.
struct OneThread;

impl any_spawner::CustomExecutor for OneThread {
	fn spawn(&self, future: any_spawner::PinnedFuture<()>) {
		tokio::task::spawn_local(future);
	}

	fn spawn_local(&self, future: any_spawner::PinnedLocalFuture<()>) {
		tokio::task::spawn_local(future);
	}

	fn poll_local(&self) {}
}

/// Every hook, against `db`.
pub async fn hooks(db: Db) {
	let _ = Executor::init_custom_executor(OneThread);
	rpc::server::Server::new(db.clone()).install();
	let live = Live::new(db.clone());
	let clients = LiveClients::new(&live);

	LocalSet::new()
		.run_until(async {
			assert!(
				clients.install(&ClientConfig::new()),
				"one client a process"
			);
			// A second configure finds the client already installed.
			rpc::client::configure(ClientConfig::new());
			let owner = Owner::new();
			owner.set();
			QueryClient::new().provide();
			let client = rpc::client::live::client();
			eventually("the connection", || client.status().is_live().then_some(())).await;

			the_default_mutation_hook_reads_its_writes(&db, &clients).await;
			pages_load_more_and_stay_live(&db).await;
			a_row_list_patches_on_screen(&db).await;
			a_live_query_whose_arguments_change().await;
			a_stream_on_screen().await;
			the_status_follows_the_connection(&clients).await;
			mutations_and_actions_over_the_connection(&db).await;
			a_channel_on_screen(&clients).await;
			a_refused_channel_on_screen().await;
			a_channel_whose_arguments_change().await;
		})
		.await;
}

/// `use_mutation(mutation_scope::<M>())` goes over the open connection, so
/// it resolves with its write already applied; without one, over HTTP.
async fn the_default_mutation_hook_reads_its_writes(db: &Db, clients: &LiveClients) {
	let client = rpc::client::live::client();
	let bodies = rpc::use_live(|| HookBodies {});
	let send = use_mutation(mutation_scope::<HookWrite>());
	eventually("the first value", || bodies.data().filter(Vec::is_empty)).await;

	send.mutate_async(HookWrite {
		body: "mine".into(),
	})
	.await
	.unwrap();
	let committed = u64::try_from(db.live_version().await.unwrap()).unwrap();
	assert!(
		client.version() >= committed,
		"resolved once the connection applied its commit: {} < {committed}",
		client.version()
	);
	eventually("the write on screen", || {
		bodies.data().filter(|bodies| bodies == &["mine"])
	})
	.await;

	// Without a connection, it goes over HTTP (here, in process) at once.
	clients.outage(true);
	eventually("the outage", || (!client.carries_calls()).then_some(())).await;
	send.mutate_async(HookWrite {
		body: "offline".into(),
	})
	.await
	.unwrap();
	clients.outage(false);
	eventually("the reconnect", || client.status().is_live().then_some(())).await;
	eventually("both writes on screen", || {
		bodies.data().filter(|bodies| bodies.len() == 2)
	})
	.await;
	let messages = Message::query().all(db).await.unwrap();
	assert_eq!(messages.len(), 2, "each ran once");
}

/// Loading more pins the last page; a write lands on page one alone.
async fn pages_load_more_and_stay_live(db: &Db) {
	for body in ["p1", "p2", "p3"] {
		write(db, body).await;
	}

	let history = rpc::use_live_paginated(|page| HookHistory { page }, 2);
	let bodies = move || {
		history
			.pages()
			.iter()
			.map(|page| {
				page.items
					.iter()
					.map(|row| row.body.clone())
					.collect::<Vec<_>>()
			})
			.collect::<Vec<_>>()
	};
	eventually("the first page", || {
		(bodies() == [vec!["p3", "p2"]]).then_some(())
	})
	.await;
	assert!(history.has_more());

	history.load_more().await;
	eventually("the second page", || {
		(bodies() == [vec!["p3", "p2"], vec!["p1", "offline"]]).then_some(())
	})
	.await;
	assert!(
		history.query().page_params()[0].until.is_some(),
		"the first page is pinned"
	);

	write(db, "p4").await;
	eventually("the new row on page one, and page two unmoved", || {
		(bodies() == [vec!["p4", "p3", "p2"], vec!["p1", "offline"]]).then_some(())
	})
	.await;
	assert_eq!(history.items().len(), 5);
}

/// A live query whose arguments change subscribes to the new ones and
/// leaves the old subscription behind.
async fn a_live_query_whose_arguments_change() {
	let limit = RwSignal::new(1_u32);
	let history = rpc::use_live(move || {
		HookHistory {
			page: rpc::PageArgs::first(limit.get()),
		}
	});
	let rows = move || history.data().map(|page| page.items.len());
	eventually("the first page size", || (rows() == Some(1)).then_some(())).await;

	limit.set(2);
	eventually("the second page size", || (rows() == Some(2)).then_some(())).await;
}

/// A channel whose arguments change leaves the old instance and joins the
/// new one.
async fn a_channel_whose_arguments_change() {
	let room = RwSignal::new("changing-a".to_owned());
	let handle = rpc::use_channel(move || HookRoom { room: room.get() });
	eventually("the first room", || {
		(handle.members().get().len() == 1).then_some(())
	})
	.await;

	room.set("changing-b".to_owned());
	eventually("the second room", || {
		(handle.members().get().len() == 1).then_some(())
	})
	.await;
}

/// A list of rows arrives as a patch to the cached value.
async fn a_row_list_patches_on_screen(db: &Db) {
	let rows = rpc::use_live(|| HookRows {});
	let bodies = move || {
		rows.data()
			.unwrap_or_default()
			.into_iter()
			.map(|row| row.body)
			.collect::<Vec<_>>()
	};
	let initial_rows = eventually("the row list", || rows.data()).await;
	let initial: Vec<String> = initial_rows.into_iter().map(|row| row.body).collect();
	assert!(
		initial.ends_with(&["p2".to_owned(), "p3".to_owned(), "p4".to_owned()]),
		"{initial:?}"
	);
	let initial = initial.len();

	write(db, "p5").await;
	eventually("the patched row", || {
		let bodies = bodies();
		(bodies.len() == initial + 1 && bodies.last().is_some_and(|last| last == "p5"))
			.then_some(())
	})
	.await;

	Message::query()
		.filter(|message| message.body.eq("p2".to_owned()))
		.delete(db)
		.await
		.unwrap();
	eventually("the removed row", || {
		let bodies = bodies();
		(bodies.len() == initial && !bodies.iter().any(|body| body == "p2")).then_some(())
	})
	.await;
}

/// A stream's items arrive on screen, and it ends.
async fn a_stream_on_screen() {
	let count = rpc::use_stream(|| HookCount { upto: 3 });
	eventually("the stream's end", || {
		(count.state().get() == StreamState::Done).then_some(())
	})
	.await;
	assert_eq!(count.items().get(), [0, 1, 2]);
	assert!(count.state().get().is_finished());

	// Every copy of the handle shows the same items.
	let copy = count;
	copy.clear();
	assert_eq!(count.items().get(), Vec::<u32>::new());

	let broken = rpc::use_stream(|| HookBroken {});
	eventually("the stream's failure", || {
		matches!(broken.state().get(), StreamState::Failed(_)).then_some(())
	})
	.await;
	assert!(broken.state().get().is_finished());
	assert_eq!(broken.items().get(), [1]);
	assert!(!StreamState::Opening.is_finished());
}

/// The status hook follows the connection down and back up.
async fn the_status_follows_the_connection(clients: &LiveClients) {
	let status = rpc::use_live_status();
	eventually("live", || status.get().is_live().then_some(())).await;

	clients.outage(true);
	eventually("the outage", || (!status.get().is_live()).then_some(())).await;
	clients.outage(false);
	eventually("the reconnect", || status.get().is_live().then_some(())).await;
}

/// The live mutation and action hooks send over the connection.
async fn mutations_and_actions_over_the_connection(db: &Db) {
	let write = rpc::use_live_mutation::<HookWrite>();
	write
		.mutate_async(HookWrite {
			body: "live".into(),
		})
		.await
		.unwrap();
	assert!(
		Message::query()
			.all(db)
			.await
			.unwrap()
			.iter()
			.any(|message| message.body == "live")
	);

	let shout = rpc::use_live_action::<HookShout>();
	assert_eq!(
		shout
			.mutate_async(HookShout { body: "hi".into() })
			.await
			.unwrap(),
		"HI"
	);
}

/// A channel's members and the others' signals reach the hook, and its
/// own signals reach the others.
async fn a_channel_on_screen(clients: &LiveClients) {
	let lobby = || {
		HookRoom {
			room: "lobby".into(),
		}
	};
	let room = rpc::use_channel(lobby);
	eventually("joined", || (room.members().get().len() == 1).then_some(())).await;

	let other = clients.client(&ClientConfig::new());
	let (key, mut events) = other.join(
		lobby(),
		Some(Seat {
			name: "grace".into(),
		}),
	);
	eventually("two members", || {
		(room.members().get().len() == 2).then_some(())
	})
	.await;

	other.signal::<HookRoom>(
		&key,
		&Wave {
			from: "grace".into(),
		},
	);
	eventually("grace's wave", || {
		(room.signals().get()
			== [Wave {
				from: "grace".into(),
			}])
		.then_some(())
	})
	.await;
	room.clear_signals();
	assert_eq!(room.signals().get(), Vec::<Wave>::new());

	// A message that doesn't fit the screen's type is dropped.
	other.signal::<HookRoomAnything>(&key, &serde_json::json!({ "not": "a wave" }));
	// The newest 100 are kept, oldest first.
	for n in 0..101 {
		other.signal::<HookRoom>(
			&key,
			&Wave {
				from: n.to_string(),
			},
		);
		let expected = usize::try_from(n + 1).unwrap().min(100);
		eventually("the newest signals", || {
			(room.signals().get().len() == expected).then_some(())
		})
		.await;
	}
	eventually("the 101st signal", || {
		room.signals()
			.get()
			.last()
			.is_some_and(|wave| wave.from == "100")
			.then_some(())
	})
	.await;
	let signals = room.signals().get();
	assert_eq!(signals[0].from, "1", "the oldest was dropped");
	assert_eq!(signals[99].from, "100");

	room.signal(Wave { from: "me".into() });
	let received = tokio::time::timeout(Duration::from_secs(20), async {
		while let Some(event) = futures::StreamExt::next(&mut events).await {
			if let rpc::client::ChannelEvent::Signal { message, .. } = event {
				return message;
			}
		}
		panic!("the channel closed");
	})
	.await
	.expect("the wave arrives");
	assert_eq!(received, serde_json::json!({ "from": "me" }));
}

/// A refused join is ignored on screen rather than shown as a move.
async fn a_refused_channel_on_screen() {
	let room = rpc::use_channel(|| {
		HookRoom {
			room: "secret".into(),
		}
	});
	// Before its effect joins, a signal has nowhere to go.
	room.signal(Wave {
		from: "early".into(),
	});
	tokio::time::sleep(Duration::from_millis(100)).await;
	assert_eq!(room.members().get(), [] as [rpc::live::PresenceMember; 0]);
	assert_eq!(room.signals().get(), [] as [Wave; 0]);
}
