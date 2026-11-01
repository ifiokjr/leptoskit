# Huddle

A team chat with a task board, built to show every real-time primitive in leptoskit at once: live queries, channels with presence, background jobs, actions, and the inbox. One server, one database, and one interface that renders in the browser and as native widgets on macOS and iOS.

Open it in two windows, sign in as two people, and type in one: the other sees the message, the unread count, and "Ada is typing…" as they happen.

## What it shows

| Feature                                                                                      | Primitive                                                                                                    | Where                                                    |
| -------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------ | -------------------------------------------------------- |
| A sidebar where a private channel appears when you're added and vanishes when you're removed | A live query whose result changes because a policy's `cx.exists(…)` on memberships changed                   | `huddle_db` (policies), `huddle_api/src/channels.rs`     |
| Unread counts that agree with the message list                                               | Two live queries updated in one transition                                                                   | `channels`, `messages`                                   |
| Sending shows at once, greyed until the server's row arrives, with Retry if it fails         | An optimistic copy keyed by the sender, matched against the live list: read-your-writes, and one row per key | `huddle_api/src/messages.rs`, `huddle_app/src/huddle.rs` |
| "Ada is typing…" and the avatars of who's here                                               | `#[rpc::channel]` signals and presence                                                                       | `huddle_api/src/room.rs`, `huddle_app/src/room.rs`       |
| A task board where two people moving cards at once end up with the same order                | A serializable `#[rpc::mutation]`, retried on conflict                                                       | `huddle_api/src/board.rs`                                |
| Invite by email: _Sending…_, then _Email sent_, and a notice                                 | A `#[rpc::job]` enqueued in the mutation's transaction, updating a live row                                  | `huddle_api/src/invites.rs`                              |
| "Remind me in 10 minutes"                                                                    | `cx.enqueue(…).after(…)`                                                                                     | `huddle_api/src/reminders.rs`                            |
| A bell with unread notices on every device                                                   | `use_inbox`                                                                                                  | `huddle_app/src/ui/bell.rs`                              |
| "Summarize this channel"                                                                     | An `#[rpc::action]` calling a `Summarizer`, then a mutation that pins the summary                            | `huddle_api/src/summary.rs`                              |
| "Tip a teammate": pending at once, then confirmed, or failed with **Retry**                  | A job with checkpointed `cx.step(…)`s, `on_failure`, and an inbox notice whose action is a mutation          | `huddle_api/src/tips.rs`                                 |
| An admin signs someone out everywhere, and their open apps drop to the sign-in screen        | `Auth::sign_out_everywhere`, and a live query on their own row                                               | `huddle_api/src/people.rs`                               |

Tips use a **simulated** settlement service that runs inside the server: it takes about two seconds and declines a fifth of tips (`HUDDLE_SETTLEMENT_DELAY_MS`, `HUDDLE_SETTLEMENT_FAILURE_RATE`), so every state is easy to see. No money, coins, or blockchain are involved. The summarizer is local and deterministic too: it counts who spoke and which words came up, and calls nothing outside.

Sign-in is the screen `leptoskit generate auth` writes, with emailed codes. Every email, sign-in codes and invitations alike, stays in the server's mailbox at `/_leptoskit/mail`. In a debug build the browser's sign-in screen also has buttons that sign in as Ada, Grace, or Lin at once. The first person to sign in is the admin.

## How it's built

```text
examples/huddle/
├── leptoskit.toml   # web, iOS, and macOS; client-rendered; SQLite; [auth]; [rpc.jobs]
├── huddle_db/       # the tables, their policies, the auth and rpc packs, and the sign-up hook
├── huddle_api/      # queries, mutations, the room, jobs, and actions, shared by the server and every client
├── huddle_app/      # one interface for the browser and the native guest, in view! markup
├── huddle_server/   # the `huddle` binary: the site, /api, /live, /auth, the job workers, /healthz and /readyz
├── load/            # huddle-load: thousands of simulated clients over the live protocol
├── migrations/      # written by `leptoskit db migration create`
├── deploy/          # written by `leptoskit generate deploy` for Fly and Compose
├── ios/, macos/     # the native hosts
└── web/index.html
```

Every table is `live`, so writes from anywhere reach the screens watching them, and every table has a policy, so a screen only ever receives rows its reader could fetch. Nothing in the app names a subscription or invalidates a cache: a write re-runs the queries it could change, as each of their readers.

The server is SQLite on one file. The tables are plain `#[db::table]` structs, so pointing `[db]` at Postgres and running `leptoskit db migration create` gives you Postgres migrations of the same schema; the committed migration is SQLite's.

## Run it

You need the leptoskit CLI (`cargo install --path crates/leptoskit_cli` from the repository root) and the `wasm32-unknown-unknown` target.

### In the browser

```bash
cd examples/huddle
leptoskit dev
```

`leptoskit dev` builds the web client and the server, migrates `huddle.sqlite3`, and serves both at `http://127.0.0.1:3000`. Open it in two windows (one private, so they keep separate cookies), press **Ada** in one and **Grace** in the other, and talk. The board, invitations, reminders, and tips are under **Board** and **Activity**; notices are under **Notices**.

### On macOS and the iOS simulator

With the server from `leptoskit dev` running:

```bash
leptoskit dev --platform macos
leptoskit dev --platform ios     # the iOS simulator
```

The native apps reach `http://127.0.0.1:3000` through the HTTP and WebSocket plugins. They sign in with an emailed code, since dev sign-in sets a browser cookie: type an address, open `http://127.0.0.1:3000/_leptoskit/mail` for the code, and type it in. A native app that was already signed in when it opened goes live at once; one that signs in while open shows the data and goes live the next time it starts (see the gaps below).

### Across machines, on Fly

`deploy/` holds what `leptoskit generate deploy --target fly` and `--target compose` wrote, adapted to build from the repository root, where Huddle's crates are. From the repository root:

```bash
fly launch --config examples/huddle/deploy/fly.toml --copy-config --no-deploy
fly volumes create data --size 1 --config examples/huddle/deploy/fly.toml
fly secrets set AUTH_ENCRYPTION_KEYS="$(date +%Y-%m):$(openssl rand 32 | basenc --base64url | tr -d =)" \
  --config examples/huddle/deploy/fly.toml
fly deploy --config examples/huddle/deploy/fly.toml --dockerfile examples/huddle/deploy/Dockerfile
```

Change `app` and `HUDDLE_ORIGIN` in `fly.toml` to a name of your own first: Fly's app names are global. The server runs one machine on a volume (SQLite), drains its sockets on `SIGTERM`, and only gets traffic once `/readyz` passes. A release build has no dev sign-in, so people sign in with a code from `https://<app>.fly.dev/_leptoskit/mail`: the demo keeps mail there on purpose (`HUDDLE_DEMO_MAIL=1`), which means anyone can read it, so don't use the demo for anything private.

Point the native apps at it by building with `HUDDLE_SERVER=https://<app>.fly.dev` and adding that origin (and its `wss://` form) to `[native.plugin-options.http]` and `[native.plugin-options.websocket]` in `leptoskit.toml`. Then open it on a laptop, a phone, and a second laptop.

`docker compose -f examples/huddle/deploy/compose.yaml up --build` runs the same container on one machine.

## Load

`huddle-load` signs in a set of teammates through dev sign-in, starts channels with all of them in, and opens live connections, each subscribed to one channel's messages over the same wire protocol the apps speak (`leptoskit_rpc::live`'s frames over `tokio-tungstenite`). It then sends messages at a steady rate, round-robin across the channels, and reports the time from commit to update: from each message's `created_at`, which the server stamps inside the transaction that commits it, to the moment each connection reads the frame carrying it. Both clocks are the same machine's, so run it beside the server.

The server needs dev sign-in, which only debug builds have; an optimized build keeps it with `CARGO_PROFILE_RELEASE_DEBUG_ASSERTIONS=true`:

```bash
# Terminal 1, from the repository root:
ulimit -n 20000
CARGO_PROFILE_RELEASE_DEBUG_ASSERTIONS=true cargo build --release -p huddle -p huddle_load
cd examples/huddle && HUDDLE_DEMO_MAIL=1 DATABASE_URL=sqlite://load.sqlite3 ../../target/release/huddle

# Terminal 2:
ulimit -n 20000
./target/release/huddle-load --clients 500 --channels 10 --accounts 100 --rate 20 --seconds 30
./target/release/huddle-load    # the defaults: 5,000 clients across 50 channels
```

On Postgres, set `DATABASE_URL=postgres://…`; the server applies `migrations-postgres/` there. For two servers behind the live relay, start a NATS server with JetStream and give both its URL, one with every role and one with the web and live roles, each on its own port, then point the load generator at both: half the connections go to each, and the messages take turns between them, so most updates cross from one server to the other through the relay.

```bash
nats-server -js &
export HUDDLE_DEMO_MAIL=1 NATS_URL=nats://127.0.0.1:4222 DATABASE_URL=postgres://localhost/huddle
LEPTOS_SITE_ADDR=127.0.0.1:3101 ../../target/release/huddle --role all &
LEPTOS_SITE_ADDR=127.0.0.1:3102 ../../target/release/huddle --role web,live &
./target/release/huddle-load --url http://127.0.0.1:3101 --url http://127.0.0.1:3102
```

Measured on 1 November 2026 on an Apple M4 Mac mini (10 cores, 16 GB, macOS 27.0.1), with the servers, Postgres 17 (default settings, `fsync` on), NATS 2.14 and the load generator all on the same machine, built as above. Each message is one `POST /api/send_message`; every connection in its channel should receive it. One hub per server reads each commit once and re-runs the queries it could change, shared between the connections that watch the same channel as the same person.

| Setup                               | Clients | Channels | Messages   | Updates received | Errors | p50    | p95     | p99      |
| ----------------------------------- | ------- | -------- | ---------- | ---------------- | ------ | ------ | ------- | -------- |
| One server, SQLite                  | 500     | 10       | 589 (20/s) | 29,450 of 29,450 | 0      | 4.8 ms | 24.8 ms | 40.8 ms  |
| One server, SQLite                  | 5,000   | 50       | 598 (20/s) | 59,800 of 59,800 | 0      | 3.1 ms | 20.4 ms | 110.8 ms |
| One server, Postgres                | 500     | 10       | 599 (20/s) | 29,950 of 29,950 | 0      | 7.2 ms | 16.4 ms | 23.3 ms  |
| One server, Postgres                | 5,000   | 50       | 601 (20/s) | 60,100 of 60,100 | 0      | 5.9 ms | 11.9 ms | 95.8 ms  |
| Two servers, Postgres, through NATS | 500     | 10       | 601 (20/s) | 30,050 of 30,050 | 0      | 5.9 ms | 8.9 ms  | 13.8 ms  |
| Two servers, Postgres, through NATS | 5,000   | 50       | 599 (20/s) | 59,900 of 59,900 | 0      | 5.7 ms | 23.1 ms | 41.4 ms  |

The slowest updates at 5,000 connections on one server ran from 0.25 s (Postgres) to 1.25 s (SQLite), and a second SQLite run's p99 was 220 ms, so the tail moves between runs; the medians don't. Before the single hub, each connection read the change log after every commit on its own, and the same 5,000 connections on SQLite took p50 1.16 s and p99 2.32 s. With two servers, the second takes every commit from the relay's stream and holds no connection to the database for it.

## Tests

```bash
cargo test -p huddle      # two people against the real server: see below
cargo test -p huddle_app  # every screen, rendered headlessly at a laptop's and a phone's width
cargo test -p huddle_api  # the summarizer
```

`huddle_server/tests/two_people.rs` starts the server in process on a migrated SQLite database and signs in Ada and Grace through dev sign-in. Over HTTP and the live socket it checks that a message reaches the other socket with its unread count in the same transition and is stored once when sent twice; that a private channel appears in Grace's sidebar when she's added and vanishes when she's removed; that typing signals and presence pass between them; that an invite goes from _Sending…_ to _Email sent_ with a notice and an email in the mailbox; that a declined tip turns _failed_ with a **Retry** notice whose action then settles it; that a job's own mutation refuses a client; and that signing Grace out everywhere reaches her open socket.

## Gaps

- **No drag gesture.** leptoskit has none yet, so cards move with buttons that call the same `move_task` a drop would.
- **Signing out everywhere doesn't close the person's sockets.** The live connection follows sign-in and sign-out on its own device (the auth client re-authenticates it, so nothing reloads), but the server doesn't yet end other devices' sockets when their session is revoked. Huddle records it on the person's own `users` row, which their `my_session` query watches, and the app signs out when it changes. Their HTTP calls are refused at once either way.
- **Presence through `LiveClient::join`.** `use_channel` joins with the default presence, so Huddle joins through the live client itself to give each avatar a name.
- **Mutations go over HTTP.** The client's hooks call mutations with `POST /api/<name>`, so read-your-writes is the app's own: each pending message carries a key the sender made, and the screen keeps its grey copy until the live list has that key.
- **History is a page.** `messages` takes `before` for older pages, but the screen shows the latest fifty; `use_live_paginated` isn't built yet.
- **`leptoskit generate deploy` refuses client-rendered apps,** even with a server of their own, so `deploy/` was generated with `render` set to `ssr`, which writes the same files, and adapted for this repository.
- **Only web, macOS, and iOS.** The same guest would run on Android, Windows, and Linux, but their hosts aren't set up here. The browser client and the guest are linted for wasm in CI. The macOS app has been built with `leptoskit build --platform macos` and opened against a local server, where it connected over HTTP and the live socket; signing in on it, and the iOS build, haven't been driven yet.
- **The hosts are workspace members.** `leptoskit build` builds a host as a member of the app's workspace, and Huddle's crates are members of the repository's, so `ios/` and `macos/` are too. Their build scripts copy the generated registrant only into the CLI's own build of them (`target/leptoskit-native/`), so the workspace builds and lints without one.
