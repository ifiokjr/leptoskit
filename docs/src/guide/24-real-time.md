# Real-time screens

A screen that reads the database shows what was true when it rendered. Someone else's message, a task moving on the board, or a colleague's edit stays invisible until the screen refetches — so apps add polling, and polling is either too slow or too expensive.

leptoskit's live queries close that gap without a second API. The database says which tables are worth watching, a query is an ordinary `#[rpc::query]`, and the screen renders the same cached value it already renders. When a write commits, the server re-runs the queries it affects and sends each client only what changed.

## Mark the tables worth watching

A table opts in, once:

```rust
use leptoskit_db::Id;
use leptoskit_db::table;

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

let app = Router::new().merge(rpc.router()).merge(live.router());
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
	let messages = use_live::<messages>(move || {
		messages {
			channel,
			limit: 100,
		}
	});

	view! {
		<View class="flex-col gap-2 p-4">
			{move || {
				messages
					.data()
					.unwrap_or_default()
					.into_iter()
					.map(MessageRow)
			}}
		</View>
	}
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

When someone sends a message, the message should be on their screen the moment the send button stops spinning, not a beat later. The ordinary mutation hook already does this whenever the app holds a live connection:

```rust
use leptoskit::rpc::client::mutation_scope;

let send = use_mutation(mutation_scope::<SendMessage>());
send.mutate(SendMessage { channel, body });
```

While the connection is open, the mutation goes over it rather than over HTTP, and it resolves only once that connection has received the update that includes the new message. By the time `is_pending()` turns false or `on_success` runs, `use_live(move || Messages { … })` already shows it. Here is how:

- Every commit to a live table gets the next number in the database's change log, its _version_.
- The mutation's answer carries the version its own commit got.
- Every update the server sends says which version it brings the screen to, and the client holds the answer until it has applied one at or past it.

A mutation that changes nothing the screen is watching still resolves promptly: the server tells the connection it has reached that version with an empty update. A mutation that fails resolves at once, with its error. When there is no open connection, as while the server renders or while the network is down, the same hook calls the mutation over HTTP, with the same idempotency key on every retry.

`use_live_mutation::<SendMessage>()` is the same hook under its older name. Actions work the same way with `use_live_action`: an action that runs mutations resolves once their writes are on screen. An action is never sent twice, so if the connection drops before it answers, it fails with a network error rather than risking running again.

## Long lists, a page at a time

A channel's history, an order queue, or a feed is too long to load at once, and it changes while someone scrolls. Paging by offset breaks as soon as a row is inserted above the reader: every later page shifts by one, and a row shows up twice or not at all. A paginated live query keeps every loaded page live and keeps its boundaries where they were.

The query takes a `PageArgs` and returns a `Page`, and `rpc::paginate` reads one page of any query:

```rust
#[rpc::query]
pub async fn history(
	cx: &rpc::QueryCx,
	channel: Id<Channel>,
	page: rpc::PageArgs,
) -> rpc::Result<rpc::Page<Message>> {
	let messages = Message::query().filter(|message| message.channel.eq(channel));
	rpc::paginate(
		cx.db(),
		messages,
		&page,
		|message| message.id.desc(),
		|message| message.id,
	)
	.await
}
```

The order must be by a unique column, such as the id (leptoskit's ids are ordered by when they were created, so `id.desc()` is newest first). Leave the query itself unordered: a row's place in that order is its cursor.

On a screen, `use_live_paginated` builds the query for each page and loads them a `page_size` at a time:

```rust
let history = use_live_paginated(move |page| History { channel, page }, 50);

let rows = move || history.items();
let older = move || {
	history.has_more().then(|| {
		view! {
			<Button on_press=move || {
				history.load_more();
			}>
				"Older messages"
			</Button>
		}
	})
};
```

Every loaded page is its own live subscription, written into one `leptoskit_query` infinite query. Loading more pins the last page at the row it ended on, the way Convex's paginated queries do. From then on:

- a row inserted into page one makes page one longer, and page two stays as it was;
- a row deleted from page one makes it shorter, without pulling a row back from page two;
- the last page reads up to `page_size` rows after the page before it, so it takes in new rows at its end until the next `load_more`.

So a row is on exactly one page however the data moves, and each page updates live, as a patch of its rows. Each page re-runs only when a write could change its own range.

This works the same on the web and in native apps. While the server renders, and on a platform with no socket, it is an ordinary infinite query that fetches each page once.

## Streams

Some server output isn't a query over rows: a model's answer arriving a token at a time, a long import's log, a file being tailed. A stream sends it over the same connection as the live queries, at the pace the screen takes it:

```rust
#[rpc::stream]
pub async fn answer(
	cx: &rpc::QueryCx,
	prompt: String,
) -> rpc::Result<impl Stream<Item = String>> {
	let me = cx.require_user::<User>()?;
	Ok(my_model::tokens(me, prompt))
}
```

The body runs as the caller, like a query. An item type of `rpc::Result<T>` ends the stream as failed at the first error. On a screen:

```rust
let answer = use_stream(move || {
	Answer {
		prompt: prompt.get(),
	}
});

let text = move || answer.items().get().concat();
let thinking = move || !answer.state().get().is_finished();
```

`use_stream` opens the stream while the component is mounted, opens it again when its arguments change, and cancels it when the component goes away, so the server stops producing. `state()` is `Opening`, `Open`, `Done`, `Failed(error)`, or `Interrupted`.

The client grants the server room for a number of items, and grants more as the screen takes them. The server reads the next item from the body only when it has room, so a phone on a slow network never makes the server hold more than it asked for.

A stream ends when the connection does. Mark it `resumable` to have it pick up again after a reconnect. It then takes a `cursor` argument, and its items say where they reached:

```rust
impl rpc::Resumable for LogLine {
	type Cursor = u64;

	fn cursor(&self) -> u64 {
		self.offset
	}
}

#[rpc::stream(resumable)]
pub async fn tail(
	cx: &rpc::QueryCx,
	file: String,
	cursor: Option<u64>,
) -> rpc::Result<impl Stream<Item = LogLine>> {
	Ok(read_from(file, cursor.map_or(0, |offset| offset + 1)))
}
```

After a reconnect, the client opens it again with `cursor` set to the last item it received, so nothing repeats and nothing is skipped. Any other stream ends as `Interrupted`. When the state could be a row, such as an import's progress, prefer a live query: it survives reconnects and server restarts on its own.

Streams work over the WebSocket and over the server-sent events fallback in the browser, and over the WebSocket in native apps. `#[api]` publishes a stream to the spec as a `stream` operation: it appears in `/asyncapi.json`, describing one item, and not in `/openapi.json`, because there is nothing to call once over HTTP. Generated clients don't open streams yet.

## Signing in and out

A screen that shows someone's private channels should show them the moment they sign in, and drop them the moment they sign out, without a reload. The live connection follows the sign-in: when `leptoskit_auth`'s client signs in, signs out, or renews a token (with its `rpc` feature on, which `lk generate auth` turns on), it tells the connection, and every live query, channel and stream reads as the new identity.

- **Native apps** (iOS, Android, desktop) send the new access token on the open connection. The server re-runs every subscription and re-checks every channel as the new identity, and sends the new results in one update.
- **In the browser**, the session is a cookie, which only travels on a new connection, so the client reconnects and takes a fresh value for every query rather than resuming.

Signing out works the same way: results the signed-out person can no longer see become empty or `Forbidden`. A stream opened as the old identity ends as interrupted; a resumable one opens again as the new one. If your app signs in some other way, call `rpc::client::reauthenticate()` after the credentials change.

Ending a session from another device also ends its open connections. With `Live::new(db).identify(auth)`, first-party cookie and bearer sessions are checked when their session or account changes; disabling an account ends all its connections. This works on WebSocket and SSE, including idle connections with no queries. Session and access-token deadlines are enforced even when the database is quiet. Native bearer clients receive a warning up to a minute before access-token expiry and renew through their configured credentials. Shorter token lifetimes warn halfway through; an absolute session deadline is enforced without repeated renewal attempts.

After upgrading an existing app, run `lk db migration create live_auth_sessions` and apply the migration: the auth pack now captures changes to accounts and sessions. Keep this migration with the rest of your app's migrations.

If you implement `Identify` yourself, implement its `lease` method to return the reads and deadlines that keep an identity valid. Make those checks against the primary database. The default lease has no dependencies or deadline; it does not validate a custom session after the connection opens.

In a leptoskit app, refused credentials fail pending live calls and cached values are fetched afresh on the next connection. Generated SDKs report refusal through their auth-refusal callback; close that client and create a fresh one when signing in as another person, so its cached subscriptions belong to that person. An action already running on the server may finish its work; if the connection has since accepted a different identity, its old result is returned as an authorization error.

## Working offline

On a phone, the network comes and goes. A message sent in a tunnel should go out when the train comes out, once, in order. Turn on the offline queue when you configure the client:

```rust
rpc::client::configure(ClientConfig::new().offline_queue(true));
```

With it on, mutations and actions dispatched while the connection is down wait, in the order they were dispatched, and go out when it comes back. The queue is also kept on the device, so what was dispatched before the app closed goes out the next time it starts:

- **In the browser**, in `localStorage`, under the page's origin.
- **In native apps**, in the store plugin's settings file. Grant the app the `store` plugin.

A mutation is replayed with the idempotency key it was dispatched with, so if its first send did reach the server before the network went, the replay runs nothing twice. An action can't be made safe to repeat, so it is kept only until it is sent: an app that closes while an action is in flight never sends it again.

`use_live_status()` shows what is happening, for a banner:

```rust
let status = use_live_status();
let banner = move || {
	let status = status.get();
	match status.state {
		LiveState::Reconnecting { .. } if status.queued > 0 => {
			Some(format!("{} changes waiting", status.queued))
		}
		LiveState::Reconnecting { .. } => Some("Reconnecting…".to_owned()),
		_ => None,
	}
};
```

`state` is `Connecting`, `Live { version }`, `Reconnecting { attempt, next_in }`, or `Offline` (no live connection in this build, as while the server renders). `queued` counts the mutations and actions still waiting for an answer.

Without the offline queue, a mutation sent while the connection is down goes over HTTP straight away, and fails if that can't get through either.

## Reconnecting

The connection reconnects with exponential backoff and full jitter, from 250 ms to 30 seconds. It **resumes** rather than starting over: it tells the server the version it had reached and the queries it holds, and the server sends one update with only the results that changed while it was away. A phone that drops off the network for a few seconds gets back a few rows, not every list on every screen.

The server keeps the change log for a while, then prunes it (`Db::prune_changes`). A client that was away longer than that gets every result again, which is always correct, just bigger. Pending mutations are re-sent with the same request id — the receipts behind `#[rpc::mutation]` make a re-send run once.

In the browser, if the WebSocket fails to connect twice in a row, the client switches to server-sent events and HTTP posts for the rest of the page's life. Nothing in the app changes: the same hooks, the same updates, the same resume. Native apps always use the WebSocket, through the WebSocket plugin, because their HTTP plugin can't stream a response.

A deployment can drain its connections: call `live.drain()` on `SIGTERM` and every connection receives `reconnect { after_ms }` with jitter, so the new instances don't all get the same reconnect storm. Because clients resume, a deploy costs each one only what changed during the restart.

## Several servers

On Postgres, run as many copies of the server as you like behind a load balancer that passes WebSockets through: any of them can resume any client, and each reads every commit from the database. To share channels between them, take commits from NATS rather than each reading the database, run sockets apart from pages and jobs, or re-run queries on read replicas, see [live apps from one server to many](16-security-and-scale.md#live-apps-from-one-server-to-many).

## Limits

| Limit                  | Default | What happens                                                 |
| ---------------------- | ------- | ------------------------------------------------------------ |
| Queries per connection | 256     | The subscription is refused                                  |
| Streams per connection | 64      | The stream ends as failed                                    |
| One result             | 512 KiB | The subscription fails with a message pointing at pagination |
| One frame              | 1 MiB   | The frame is refused                                         |

A slow connection doesn't cost the server memory without bound: `OUTGOING` transitions queue, and past that the connection gets whole values rather than every step.

## End to end

The database capture, the invalidation feed, and the policy check are described in [The database](22-database.md). The socket adds:

- `Live::new(db).identify(auth).origin(…)`, serving `GET /live`, `GET /live/events` and `POST /live/frames` (`leptoskit_rpc`, feature `live`; `rpc-live` through the `leptoskit` crate).
- `rpc::use_live::<Query>(args)`, `rpc::use_live_paginated(args, page_size)`, `rpc::use_stream(args)`, `rpc::use_live_status()`, `rpc::use_live_mutation::<Mutation>()` and `rpc::use_live_action::<Action>()`, available with the `web` or `guest` feature. `use_mutation(mutation_scope::<Mutation>())` goes over the live connection too while it is open.
- `Live::stats()`, for metrics: connections, subscriptions, re-runs, commits read, transitions sent and resumes.
- `leptoskit_query`'s cache, unchanged, so every screen and devtools see the updates.
- For apps that outgrow capture triggers, capture by Postgres's logical replication, `[db] live_capture = "replication"`, which [The database](22-database.md#capture-changes-from-the-write-ahead-log) describes. Nothing on this page changes with it.

## The live spec

An app that publishes its RPC with `#[api]` already gets `/openapi.json` for the one-shot operations. `leptoskit_rpc::spec::openapi_router` also serves `/asyncapi.json`: an AsyncAPI 3.0 document describing the live side, where a query or a stream is a `receive` operation and a mutation or action is a `send` one, with each operation's arguments and result (a stream's: one item) as message payloads. Event-driven tooling can read it the way OpenAPI tooling reads the HTTP side.

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
