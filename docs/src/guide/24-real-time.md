# Real-time screens

A screen that reads the database shows what was true when it rendered. Someone else's message, a task moving on the board, or a colleague's edit stays invisible until the screen refetches — so apps add polling, and polling is either too slow or too expensive.

leptoskit's live queries close that gap without a second API. The database says which tables are worth watching, a query is an ordinary `#[rpc::query]`, and the screen renders the same cached value it already renders. When a write commits, the server re-runs the queries it affects and sends each client only what changed.

## Mark the tables worth watching

A table opts in, once:

```rust
use leptoskit_db::{Id, table};

#[table(name = "messages", live)]
pub struct Message {
	pub id: Id<Self>,
	pub channel: ChannelId,
	pub body: String,
}
```

Only writes to live tables are captured. That is deliberate: capture costs work on every insert and update, so a table nobody watches shouldn't pay for it.

Changes publish **when the transaction commits**, not when a statement runs. A transaction that writes three rows and rolls back tells nobody. Each change carries the table and the changed rows' keys, never their contents, and only reaches a client whose policy lets them read the row before or after the change.

## Write the query

A live query is a normal query. Declare it with `#[rpc::query]` and read as the caller:

```rust
#[rpc::query]
pub async fn messages(
	cx: &rpc::QueryCx,
	channel: ChannelId,
	limit: u32,
) -> rpc::Result<Vec<PublicMessage>> {
	Message::query()
		.filter(|message| message.channel.eq(channel))
		.order_by(|message| message.created_at.asc())
		.limit(limit)
		.all(cx.db())
		.await
}
```

There is one operation, not two. The same query answers `POST /api/messages`, runs in process while the server renders, and can be subscribed to over the socket. Its policies are the same in all three, because the server runs it with the connection's caller every time.

## Serve the socket

Build the live server from the same database and the same way of identifying callers as the RPC:

```rust
use axum::Router;
use leptoskit_rpc as rpc;

let rpc = rpc::server::Server::new(db.clone()).identify(auth.clone());
let live = rpc.live().origin("https://chat.example.com");

let app = Router::new()
	.merge(rpc.router())
	.merge(live.router());
```

`GET /live` is a WebSocket, speaking the `leptoskit.live.v1.json` subprotocol. Browsers authenticate with the session cookie that rides the upgrade request; the `Origin` check refuses a page from another site. Native apps, which can't set headers on a socket, send `auth { token }` as their first frame and are identified by the same `Identify` implementation the HTTP routes use.

The same router also serves the connection over plain HTTP, for networks that won't pass a WebSocket, such as some corporate proxies:

- `GET /live/events` streams the server's messages as server-sent events.
- `POST /live/frames?connection=…` takes the client's messages, a JSON array at a time.

Both are checked like the socket: the session cookie and the `Origin` check, or a bearer token in an `auth` message. A post must come from the same caller as the stream it names. You don't call these yourself: the browser client switches to them on its own (see [Reconnecting](#reconnecting)), and the server runs the same session behind either, so every query, mutation, action and channel works the same way.

## Watch from a screen

Call `use_live` where you would call `use_rpc_query`:

```rust
use leptoskit::prelude::*;
use leptoskit::rpc::use_live;

#[component]
pub fn Channel(channel: ChannelId) -> impl IntoView {
	let messages = use_live::<messages>(move || messages {
		channel,
		limit: 100,
	});

	View().class("flex-col gap-2 p-4").children(
		move || messages.data().unwrap_or_default().into_iter().map(MessageRow),
	)
}
```

`use_live` is `use_rpc_query` plus a subscription. The first value comes from the ordinary client — over HTTP, or in process while the server renders — so suspense, SSR and hydration work as they do for any other query. The updates then arrive from the socket and are written into the same `leptoskit_query` cache, so every screen reading that query updates at once, and devtools shows it like any other query.

On a platform with no socket — a server render, or a build without the `web` or `guest` feature — `use_live` is exactly `use_rpc_query`.

## What arrives

Each query records the tables, rows, and conditions it read. When a commit arrives, the server checks those read sets against the changed rows and re-runs only the queries the write could change; a condition the matcher can't decide re-runs rather than risks a stale screen. Only results that changed are sent.

The server does that work once, not once per screen:

- **One reader per server.** Each server reads every commit from the database once, however many people are connected, and hands it to every connection that cares.
- **Shared re-runs.** When many connections watch the same query with the same arguments as the same caller, such as a dashboard open in several tabs, the server re-runs it once per commit and sends each of them the same change. Anonymous visitors with no session share too. Different people never share, because their policies may let them see different rows.

Two shapes of change:

- **A list of rows with an `id`** travels as a patch: the ids removed, and the rows inserted or replaced, with their positions. A 2,000-message channel's update stays about one message's size.
- **Everything else** travels whole.

Each transition carries the version it reaches. The client applies a whole transition before rendering, so two queries on one screen never show different versions of the database — a channel's unread count can't disagree with its messages.

If a connection falls behind, the server drops intermediate transitions and sends the current state as whole values. The client is never left applying a patch to a result it didn't have; it either has the row or receives the whole list again.

## Your own writes

When someone sends a message, the message should be on their screen the moment the send button stops spinning, not a beat later. Run the mutation with `use_live_mutation` instead of `use_mutation`:

```rust
use leptoskit::rpc::use_live_mutation;

let send = use_live_mutation::<SendMessage>();
send.mutate(SendMessage { channel, body });
```

It goes over the same connection as the live queries, and it resolves only once that connection has received the update that includes the new message. By the time `is_pending()` turns false or `on_success` runs, `use_live(move || Messages { … })` already shows it. Here is how:

- Every commit to a live table gets the next number in the database's change log, its _version_.
- The mutation's answer carries the version its own commit got.
- Every update the server sends says which version it brings the screen to, and the client holds the answer until it has applied one at or past it.

A mutation that changes nothing the screen is watching still resolves promptly: the server tells the connection it has reached that version with an empty update. A mutation that fails resolves at once, with its error.

Actions work the same way with `use_live_action`: an action that runs mutations resolves once their writes are on screen. An action is never sent twice, so if the connection drops before it answers, it fails with a network error rather than risking running again.

Without a live connection, such as while the server renders, both are the ordinary call over HTTP.

## Reconnecting

The connection reconnects with exponential backoff and full jitter, from 250 ms to 30 seconds. It **resumes** rather than starting over: it tells the server the version it had reached and the queries it holds, and the server sends one update with only the results that changed while it was away. A phone that drops off the network for a few seconds gets back a few rows, not every list on every screen.

The server keeps the change log for a while, then prunes it (`Db::prune_changes`). A client that was away longer than that gets every result again, which is always correct, just bigger. Pending mutations are re-sent with the same request id — the receipts behind `#[rpc::mutation]` make a re-send run once.

In the browser, if the WebSocket fails to connect twice in a row, the client switches to server-sent events and HTTP posts for the rest of the page's life. Nothing in the app changes: the same hooks, the same updates, the same resume. Native apps always use the WebSocket, through the WebSocket plugin, because their HTTP plugin can't stream a response.

A deployment can drain its connections: call `live.drain()` on `SIGTERM` and every connection receives `reconnect { after_ms }` with jitter, so the new instances don't all get the same reconnect storm. Because clients resume, a deploy costs each one only what changed during the restart.

## Limits

| Limit                  | Default | What happens                                                 |
| ---------------------- | ------- | ------------------------------------------------------------ |
| Queries per connection | 256     | The subscription is refused                                  |
| One result             | 512 KiB | The subscription fails with a message pointing at pagination |
| One frame              | 1 MiB   | The frame is refused                                         |

A slow connection doesn't cost the server memory without bound: `OUTGOING` transitions queue, and past that the connection gets whole values rather than every step.

## End to end

The database capture, the invalidation feed, and the policy check are described in [The database](22-database.md). The socket adds:

- `Live::new(db).identify(auth).origin(…)`, serving `GET /live`, `GET /live/events` and `POST /live/frames` (`leptoskit_rpc`, feature `live`; `rpc-live` through the `leptoskit` crate).
- `rpc::use_live::<Query>(args)`, `rpc::use_live_mutation::<Mutation>()` and `rpc::use_live_action::<Action>()`, available with the `web` or `guest` feature.
- `Live::stats()`, for metrics: connections, subscriptions, re-runs, commits read, transitions sent and resumes.
- `leptoskit_query`'s cache, unchanged, so every screen and devtools see the updates.

## The live spec

An app that publishes its RPC with `#[api]` already gets `/openapi.json` for the one-shot operations. `leptoskit_rpc::spec::openapi_router` also serves `/asyncapi.json`: an AsyncAPI 3.0 document describing the live side, where a query is a `receive` operation and a mutation or action is a `send` one, with each operation's arguments and result as message payloads. Event-driven tooling can read it the way OpenAPI tooling reads the HTTP side.

[Huddle](../examples.md#huddle), a team chat with a task board, puts every piece here together — live queries, channels and presence, typing indicators, background jobs, and the same interface in the browser and as native apps — with a test that runs two people against its server.

## Channels and presence

Some things are worth showing while they happen and not worth keeping: who is typing, who is looking at a board, a cursor moving. A channel carries those messages between the connections watching the same thing, with presence.

Declare the join arguments, the message type, the presence each member shows, and the check that decides who may join:

```rust
#[rpc::channel(message = Typing, presence = Present, authorize = may_join)]
pub struct Chat {
	pub channel: Id<Channel>,
}

async fn may_join(cx: &rpc::QueryCx, chat: &Chat) -> rpc::Result<()> {
	let me = cx.require_user::<User>()?;
	Member::query()
		.filter(|member| member.channel.eq(chat.channel) & member.user.eq(me))
		.one(cx.db())
		.await?;
	Ok(())
}
```

`authorize` runs as the caller when the connection joins, and again whenever the connection's identity changes, so signing out or losing a membership takes effect on an open socket.

Join from a screen:

```rust
let chat = use_channel::<Chat>(move || Chat { channel });

// Who is here, with the presence each joined with.
let members = chat.members();

// "Ada is typing…", kept newest first (the last 100).
let typing = chat.signals();
chat.signal(Typing { who: me.clone() });
```

Signals go to every other member of the same join instance, at most once, and are never stored. A reconnect rejoins every channel automatically. Presence is a full list on each join or leave, so a missed frame is corrected by the next one.

Channels are not a replacement for live queries: a message that must survive a restart belongs in a table, and a live query shows it.

## Background jobs

Slow work that nobody should wait for, such as sending an invite or settling a payment, runs as a background job, enqueued in a mutation's transaction. Its progress and result are rows in a live table, so `use_job` follows it over the same socket, and the inbox tells people when it finishes. [Background work](24-background-work.md) covers jobs, actions, and the inbox.
