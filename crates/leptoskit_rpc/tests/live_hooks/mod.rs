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
			let owner = Owner::new();
			owner.set();
			QueryClient::new().provide();
			let client = rpc::client::live::client();
			eventually("the connection", || client.status().is_live().then_some(())).await;

			the_default_mutation_hook_reads_its_writes(&db, &clients).await;
			pages_load_more_and_stay_live(&db).await;
			a_stream_on_screen().await;
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

/// A stream's items arrive on screen, and it ends.
async fn a_stream_on_screen() {
	let count = rpc::use_stream(|| HookCount { upto: 3 });
	eventually("the stream's end", || {
		(count.state().get() == StreamState::Done).then_some(())
	})
	.await;
	assert_eq!(count.items().get(), [0, 1, 2]);
}
