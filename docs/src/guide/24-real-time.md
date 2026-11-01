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

A change makes the server re-run the connection's queries. Only results that changed are sent. Two shapes:

- **A list of rows with an `id`** travels as a patch: the ids removed, and the rows inserted or replaced, with their positions. A 2,000-message channel's update stays about one message's size.
- **Everything else** travels whole.

Each transition carries the version it reaches. The client applies a whole transition before rendering, so two queries on one screen never show different versions of the database — a channel's unread count can't disagree with its messages.

If a connection falls behind, the server drops intermediate transitions and sends the current state as whole values. The client is never left applying a patch to a result it didn't have; it either has the row or receives the whole list again.

## Reconnecting

The socket reconnects with exponential backoff and full jitter, from 250 ms to 30 seconds. On reconnect it sends every subscription again and re-sends pending mutations with the same request id — the receipts behind `#[rpc::mutation]` make a re-send run once.

A deployment can drain its sockets: call `live.drain()` on `SIGTERM` and every connection receives `reconnect { after_ms }` with jitter, so the new instances don't all get the same reconnect storm.

## Limits

| Limit                  | Default | What happens                                                 |
| ---------------------- | ------- | ------------------------------------------------------------ |
| Queries per connection | 256     | The subscription is refused                                  |
| One result             | 512 KiB | The subscription fails with a message pointing at pagination |
| One frame              | 1 MiB   | The frame is refused                                         |

A slow connection doesn't cost the server memory without bound: `OUTGOING` transitions queue, and past that the connection gets whole values rather than every step.

## End to end

The database capture, the invalidation feed, and the policy check are described in [The database](22-database.md). The socket adds:

- `Live::new(db).identify(auth).origin(…)`, serving `GET /live` (`leptoskit_rpc`, feature `live`; `rpc-live` through the `leptoskit` crate).
- `rpc::use_live::<Query>(args)`, available with the `web` or `guest` feature.
- `leptoskit_query`'s cache, unchanged, so every screen and devtools see the updates.

A complete app — channels, presence, typing indicators, background jobs, and the same guest on the web and native — is built up in the next chapters as those primitives land.

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
