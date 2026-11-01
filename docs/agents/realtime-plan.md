# Plan: real-time data and background work in `leptoskit_rpc`

Status: **approved** (revision 3.1). Implementation is starting; each phase lands as its own pull request. This plan replaces §8 "Live data" of [database-plan.md](database-plan.md) and the "live invalidation" part of its phase 6.

Revision 2 makes live operations kinds of `#[api]` operation, in the same spec and SDKs as the RPC (§8).

Revision 2.1 records four decisions:

- writes are captured by triggers that migrations generate (R2);
- the commit-order lock's throughput ceiling is accepted for v1, with logical replication as the way past it (R3);
- tables opt in with `live` (R6);
- `lk generate deploy` ships first, as its own stack (R11).

Revision 3 makes two changes:

- **No new crate.** Everything about operations lives in `leptoskit_rpc`: the new kinds, the transports, the client, the hub and the jobs. Only change capture, which is schema work, lives in `leptoskit_db` (§2, R12).
- **Background work.** Durable jobs that report progress and results live, plus an in-app inbox to tell people when slow work finishes or fails (§3.7, §3.8, R13 to R15).

Revision 3.1 decides R13 to R15 as recommended, and builds on the database stack's `cx.count(…)` and `cx.exists(…)` policy subqueries (#305) instead of adding a separate `in_query`.

**As built, 31 October 2026.** L2 and the first half of L3 are in: `#[rpc::query]` and `#[rpc::mutation]` register operations, run through `QueryCx`/`MutationCx` with serializable retries and receipts, and serve `POST /api/<name>`; `rpc::server::Live` serves `GET /live` with subscriptions, keyed patches, limits, the `Origin` check, heartbeats and `drain()`; and `rpc::use_live` watches a query through the browser or guest WebSocket, reconnects with backoff, and writes updates into the `leptoskit_query` cache. Change capture is now the plan's (§4, R2, R3): migrations give every live table capture triggers that write the `live` pack's `live.changes` in the writing transaction, and each commit takes a version in `live.commits` (Postgres: the deferred `live.seal()` under the commit lock, then `NOTIFY` with the version; SQLite: the pool, just before `COMMIT`). #318's consumer API stays on top of it: `Db::changes()` returns a `Feed` that wakes on `NOTIFY` or the SQLite pool, reads the log after its version, and yields the same policy-filtered `Invalidation`s, now with `next_versioned()`, `Feed::version()` and `Db::changes_since(v)` to resume; a lost `LISTEN` connection or a lagging subscriber catches up from the log, and `Everything` means only a pruned log or an unknown table. `Db::change_log(v)` (system handles only) gives a server every commit with keys and images, and `ChangeLog::next_ready()` hands over whatever the same read already fetched, so a reader can take commits in batches. `Db::prune_changes(retention)` keeps a marker commit so a reader behind it gets `Everything`; `Db::live_maintenance(retention, interval)` runs it on a schedule (below). Capture differs from §4.3 in two ways: `live.capture()` and `live.seal()` only write the pack's tables (a transaction-local setting replaces `ON CONFLICT` and the `pending` delete, and pruning clears `pending`), so `SERIALIZABLE` mutations don't conflict through them; and every written row is recorded, a no-op update included. An actor is recorded only for writes in a transaction, and RPC mutations set the origin to their name. Read sets and `Expr::eval` are built (L0, #381): `leptoskit_db::Read` holds every read, `Connection` extends it with writes, `QueryCx::db()` is a `Tracked<Db>` that implements only `Read`, and `Expr::eval` decides a condition on one row soundly against both databases. A query run by name hands its read set to `rpc::server::ReadSink` through `Invocation::reads`. The hub matches each commit from `Db::change_log(…)` against every subscription's read set with `ReadEntry::could_change`, and re-runs only the queries a write could change; a condition `Expr::eval` can't decide re-runs. A test pins that a write to one live table leaves a subscription to another alone. Channels with presence are built (`#[rpc::channel]`, `use_channel`, `Live` relaying signals and re-checking `authorize` when the identity changes). L5 is built except the `lk dev` jobs page. Jobs have checkpointed `cx.step(…)`, `cx.key()` per job and step, `cx.mutate(…)` with receipts keyed by job, step and call count, `rpc::Error::Retry`/`Permanent`, `on_failure` mutations taking `rpc::Failed<J>` (run once by a sweep, held to once by their receipt), `retries`, `backoff = exponential(…)` with jitter, `deadline`, `timeout`, queues, `serialize_by`, `visible_to`, progress, cancellation (`rpc::cancel`, the `rpc.cancel_job` mutation), retention pruning, `#[rpc::cron]` (the expression parsed at compile time, ticks keyed so instances enqueue each once), `[rpc.jobs]` configuration, `lk jobs list|retry|cancel`, and `rpc::testing::TestJobs` with a manual clock and a kill-in-step fault. Two departures from §3.7: claiming is a compare-and-swap on the attempt count rather than `FOR UPDATE SKIP LOCKED`, and a job in a limited queue or a series is claimed in a `SERIALIZABLE` transaction that checks the queue's running count and the series, which Postgres's serialization checks keep correct across instances and SQLite's single writer keeps correct by construction; and workers wake on the live change feed for `rpc.jobs` (Postgres `NOTIFY`, in process on SQLite) rather than a dedicated channel. `rpc.jobs` and `rpc.inbox` are live tables with policies (owner, `visible_to`; each account its own notices), so `use_job` and `use_inbox` update over the socket and `use_job` polls only where there is none. Job keys stay unique until the job is pruned, not only while it is queued or running, and higher priorities run first. Actions are built (`#[rpc::action]`, `ActionCx` with `query`, `mutate`, `enqueue`, `notify`, `Kind::Action`, `ApiKind::Action` in the spec and OpenAPI, `use_action`), over HTTP and in process; the socket doesn't carry `act` frames yet. The inbox is built (`cx.notify` from mutations in their transaction, jobs and actions; `NoticeAction`; `use_inbox` with `mark_read` and `mark_all_read`; the `Deliver` trait and `deliver_with`, with no push channel). `lk jobs` writes bypass change capture, so a retried or cancelled job reaches watching screens at the next write to its row. The generated SDKs' live side is built (L6): `#[api]` above `#[rpc::channel]` publishes a channel, and spec version 3 adds `ApiSpec::channels` (join, message and presence schemas, and the join arguments' field order, so a generated client names a room exactly as `serde_json::to_string` of the struct does) and `ApiSpec::live_path`; AsyncAPI describes channels as `join`/`signal` sends and `signals`/`presence` receives. TypeScript (`live.ts`, global `WebSocket`), Python (`live.py`, `websockets` behind a `live` extra, a background loop for the sync client), Go (`live.go`, gorilla/websocket), Rust (`live.rs`, `tokio-tungstenite` behind a default `live` feature) and Dart (`lib/src/live.dart`, `web_socket_channel`) each carry one runtime: hello, bearer `auth` re-sent on `expiring`, client pings with a two-missed-heartbeats timeout, full-jitter backoff (250 ms to 30 s) or the server's `reconnect.after_ms`, then every sub, join and pending mutation re-sent with the same request ID and key; transitions apply atomically; and a live mutation resolves when a transition or a fresh subscription value reaches `done.version`, or after a 2 s grace period, because today's `done.version` is the hub's counter plus one rather than the commit's exact version and a receipt replay commits nothing. Subscriptions and calls share one id counter, since a subscription's first value arrives as `done`. The CLI gets `--watch` (NDJSON) and a `<channel>-join` command. `tests/sdk_live.rs` runs each language's client against a real `Live` on SQLite through a proxy that cuts the connection after a marked mutation commits, checking resume, the re-send, and that the write ran once. The SDKs don't fall back to SSE or send `act` frames yet, and streams and the relay are not built. **`examples/huddle` (§12) is built** on the public API only: every table live with membership policies (`cx.exists`), unread counts beside the messages in one transition, sending with an app-made key the sender's screen matches against the live list (the client's hooks send mutations over HTTP, so read-your-writes is the app's), typing and presence as `#[rpc::channel]` (joined through `LiveClient::join`, since `use_channel` joins with the default presence), a board whose moves are serializable mutations (buttons, as there's no drag gesture), invites and tips as jobs with steps, `on_failure` and inbox notices whose action is `retry_tip`, a reminder through `.after(…)`, a summarizing action, and an admin's sign-out-everywhere. Tips settle against a simulated in-process service instead of Solana devnet. Its test runs two people against the server in process over HTTP and the socket; `huddle_app`'s screens render headlessly; `deploy/` is `lk generate deploy`'s Fly and Compose output, generated with `render = "ssr"` because the generator refuses client-rendered apps with a server of their own. Two gaps showed up: a socket keeps the identity it connected with, so the web app reloads after signing in or out, and sign-out-everywhere reaches open sockets only through an app row (§7's revocation isn't built); and with one feed per connection, `huddle-load` (`examples/huddle/load`) measured commit-to-update at p50 13.5 ms, p99 53 ms for 500 clients across 10 channels, but p50 1.16 s, p99 2.32 s for 5,000 across 50, every update delivered, on an M4 Mac mini with SQLite; these measurements preceded the single-feed hub of §5.2, whose replacement numbers are in L7 below.

**L6 live core, 1 November 2026.** The hub of §5.2 is built: one `change_log` reader per `Live` (a system handle) takes each batch of commits the log read fetched together, finds candidate subscriptions through an index from table to group, matches them with `ReadEntry::could_change`, and re-runs only what a write could change; `LiveStats::commits` counts each commit once whatever the connection count. Subscriptions share a group, and so one re-run and one diff per batch, when the query name, the arguments (as JSON text) and the identity fingerprint match. The fingerprint is deliberately narrower than §5.4's rendered SQL: the caller plus the context's actor, or, with no caller, a context with no globals at all (`Context::is_empty`); anything else runs alone. A round applies every result under the hub's lock, queues one batch per connection, and only then moves `applied`; a group registered while a round runs is checked against the round's commits before it applies, and a subscription whose first run saw a round apply under it runs again (three times, then registers and the hub re-runs it at once). §5.5's one `REPEATABLE READ READ ONLY` snapshot per batch is not done: re-runs go through `Invocation`, which takes a `Db` and runs each query on the pool as its own identity, and a shared snapshot would need `QueryCx` over a transaction and per-statement identity switching on Postgres. Transitions therefore promise that their results reflect _at least_ their version; a commit a re-run already saw re-runs it again, harmlessly. `Session` no longer knows its transport: it reads `ClientFrame`s from a channel and writes frames to another, owns heartbeats, drain, limits, identity, channels and resume, and is the only writer of its transitions, so a `done` always precedes the transition its own write causes. `GET /live` (WebSocket, unchanged on the wire) and the SSE pair `GET /live/events` and `POST /live/frames?connection=<id>` are thin adapters; an SSE connection's name carries 128 random bits, posts pass the `Origin` check and must identify as the stream's caller unless they start with `auth`, and transitions carry their `to` as the event `id`. Read-your-writes is exact: `Tx::commit_version()` returns the commit's version (SQLite: from the seal; Postgres: the capture trigger's registered setting says whether it wrote, and `live.commits` is read by `txid` after `COMMIT`), mutations record it in a task-local that `recording_commits` reads, receipt replays report the log's newest version, and `done.version` is that commit (or the connection's last sent version when nothing live was written). A session waiting on a version sends an empty transition once the hub has applied it, and idle sessions catch up at each heartbeat, so a client's resume point stays recent. The counter-based versions are gone. `act` frames run actions in their own task with the HTTP route's identity and errors, and answer with the newest commit their mutations made. `hello.resume` gained `queries`: the server re-subscribes them without a `done` each, reads the log from the client's version (or SSE's `Last-Event-ID`, whichever is later), and sends one transition with only the results a change could have touched, or a `full` one when the log was pruned past it, holds over 10,000 commits since, or can't be read in five seconds. Members resumed as unchanged hold a value the hub never saw, so their group's next change sends them the whole value. The client resolves `mutate` and `act` once it has applied a transition at or past `done.version`, takes its version only from transitions, sends `auth` before `hello` so queries resume as the right identity, resumes subscriptions that have a value and re-subscribes the rest, fails sent actions when the connection drops instead of resending them, and in the browser switches to an `EventSource` plus `POST` transport after two failed upgrades in a row (native guests stay on the WebSocket). `use_live_mutation` and `use_live_action` route through it; `use_mutation(mutation_scope::<M>())` still calls over HTTP. Resume trusts that the client's cached results were read as the identity it reconnects with. The database half of L7 is built (1 November 2026). `[db] live_capture = "replication"` (Postgres only; `leptoskit_db::LiveCapture`, `with_live_capture`, `LEPTOSKIT_DB_LIVE_CAPTURE`) makes migrations drop the capture triggers and the `seal` constraint trigger and create `PUBLICATION leptoskit_live` over the live tables with `REPLICA IDENTITY FULL`; the diff moves between modes both ways. Publications carry no column lists: Postgres refuses updates when a column list leaves out any column of a `FULL` replica identity, and `changed` needs the secret columns anyway, so the capture task strips `secret` and `live = skip` columns itself, reading each table's key and copied columns from the publication's comment (JSON), not from its own build. `Db::capture_changes()` (`Capture`, `CaptureSettings`, `CaptureStats`) runs on every server: each holds a dedicated connection, one wins `pg_try_advisory_lock(CAPTURE_LOCK)` and does all its work on that connection, so losing it hands over to the next within `standby`. The leader holds slot `leptoskit_live_<database>` (per database, since slots are cluster-wide and branch databases share a cluster), created with `pgoutput` if missing, which also writes a `leptoskit.pruned` marker for the writes between the migration and the slot; reads `pg_logical_slot_peek_binary_changes(slot, flush_lsn, n, proto_version 1, messages true)`; decodes `Begin`, `Relation`, `Insert`, `Update`, `Delete`, `Truncate`, `Message` and `Commit`, turning text values into what `to_jsonb` would write; writes `live.commits` (with the commit LSN in the new `lsn` column and the 64-bit xid) and `live.changes` under the same commit lock with `nextval('live.version_seq')`, in commit order, then `NOTIFY`, in one transaction; and only then `pg_replication_slot_advance`, to the read's flush target when it read everything. A leader that dies between those skips commits at or before the newest recorded LSN. A `TRUNCATE`, or a table the comment doesn't describe, records a marker (`Everything`). Actor and origin come from a transactional `pg_logical_emit_message('leptoskit_live', {actor, origin})` that `Tx::commit` runs before `COMMIT` when the transaction set either and the database publishes; raw SQL that sets the settings itself is recorded without them, and generated columns are missing from images (pgoutput doesn't send them). With triggers, the capture task checks every `idle` for a leftover slot, marks a gap and drops it. `Db::live_stats()` gives `live_capture_lag` (bytes between the current WAL position and the slot's confirmed one) and `slot_active`; `Db::live_caught_up()` waits until the slot has passed the WAL insert position. Capture is asynchronous, so a feed started right after a write may still hear it; `Tx::commit_version()` waits (up to five seconds) for the slot to pass its commit, then reads the version capture gave its transaction ID. On Postgres, `live.commits` is range-partitioned by version and `live.changes` by `seq`, both known when a row is written in either mode (a trigger writes a change before its commit has a version; time isn't monotone with commit order for a long transaction's changes), with a default partition; migrations rebuild an unpartitioned log (`Step::Repartition`), and `unique(txid)` became an index. `Db::live_maintenance` (and `Db::maintain_changes` for one round) makes ranges above the default partition's rows, sized to a quarter of the retention at the measured rate, then prunes: marks the newest old commit in place (dropping its changes), drops commits ranges wholly below it and deletes default-partition commits below it, then drops each changes range none of whose rows joins a remaining commit, checked after `LOCK TABLE live.changes` so in-flight writers finish first; each step is its own transaction under `pg_try_advisory_xact_lock(PRUNE_LOCK)` with a one-second `lock_timeout`. SQLite and an unpartitioned Postgres log keep the batched delete. `rpc::server::Jobs` starts maintenance (`live_retention`, 15 minutes; `[db] live_retention_minutes`, `Jobs::from_configs`) and, with the `postgres` feature, capture (`.capture(false)` opts out). `lk db check --live` checks the publication and `wal_level` and reports the lag. Tests: `tests/live.rs` runs every Postgres case and the property in both modes; `tests/replication.rs` covers parity with triggers (images, keys, `changed`, actor, origin, raw SQL, cascades), LSN order, an open transaction captured after a later one, rollbacks, injected failures before commit and between commit and advance, two servers with failover and a killed connection, migrations switching modes both ways, partition creation and drop-based pruning in both modes, scheduled maintenance, and the concurrent-writer stress test; `leptoskit_db_schema`'s render and apply scenarios cover publications and repartitioning. Not built here: the streaming replication protocol (tokio-postgres has none; the SQL interface re-decodes from `restart_lsn` on each read, so a long-open transaction makes reads slower). The relay, `--role`, replica-aware re-runs and published numbers follow.

**L7 scale, 1 November 2026.** The rest of L7 is built. **The relay** (`rpc::server::Relay`, feature `relay` on `async-nats`, `rpc-relay` on the facade, `[rpc.live] relay_url_env`): `Relay::publish(&db)` competes for a lease (key `publisher` in the memory key-value bucket `leptoskit_live_<database>`, `max_age` = `lease`, 5 s, renewed every third of it, deleted on `stop`), and the holder reads `Db::change_log` from the stream's last message and publishes each batch the log read fetched to subject `leptoskit.live.<database>` on stream `LEPTOSKIT_LIVE_<database>` (`max_age` = retention, 15 minutes). A message is `{from, to, entries}`: `from` is the previous message's `to`, `Nats-Msg-Id` is `to`, and it's published with `Nats-Expected-Last-Sequence`, so a second publisher can't interleave (it gets `WrongLastSequence` and resyncs from the stream); batches split at `max_message_bytes` (512 KiB), and a commit bigger than that goes as `entries: null`. `Live::relay(&relay)` makes the hub's source a relay `Feed`: a pull-based ordered consumer from the stream's last sequence when the hub starts, which skips what it has, takes a message whose `from` is at or below its version, reads the gap from the database first when `from` is ahead (a reconnect past retention, a purged stream, or a restarted publisher), reads `entries: null` from the database, and, after `stall` (2 s) with no message while `Db::live_version()` stays ahead, reads the database directly until messages come again. It reads those gaps with the new `Db::change_log_after(v)`, one page without listening, as `changes_between` (resume) now does too, so a relay consumer holds no `LISTEN` connection. Commits now serialize (`Commit`, `Change`, `LogEntry`). Channels cross servers over core NATS (`leptoskit.channels.<database>`, `live/cluster.rs`): signals at most once, each server's own members of a channel on every change and all of them every 10 s, a server unheard for 30 s dropped, and presence frames listing every server's members; without a relay they stay per server, as before (the plan's `NOTIFY` fan-out for channels was never built). `LiveStats` gained `relay_fills`. **Roles** (`rpc::server::Roles`, `Role`, `--role`/`LEPTOSKIT_ROLE`, `all` by default): apps gate their own parts; huddle and the scaffold's database templates serve their routes only with `web`, huddle mounts `/live` with `live`, starts `Jobs` with `worker` (`.capture(roles.capture())`), runs `Db::capture_changes()` alone for `capture` without `worker`, and the publisher with `capture` and a relay, and `/readyz` gains a `live` check that passes after `Live::start()` (new: starts the hub and joins the relay). `lk generate deploy --target kubernetes --split-roles` (Postgres only) writes `<app>-web`, `-live`, `-worker` and `-capture` Deployments with `LEPTOSKIT_ROLE`, Services and budgets for web and live, and the relay's and replicas' URL variables among the Secret's; it also fixes the backticked `kind: PodDisruptionBudget` #445 left in every manifest. **Replicas** (`Db::with_replica(url)`, `with_replicas_from_env`, `[db] replica_url_envs`, `on_replica`, `on_primary`, `on_replica_at(lsn, wait)`, `reading_like`, `wal_position`, `replica_position`, `replica_stats`): a replica handle sends only `SELECT`s to the standby (an `INSERT … RETURNING` reads too), so writes and transactions stay on the primary, and ordinary reads stay on the primary unless the app asks. Each `Commit` carries `lsn`, a position a replica must replay to: with replication capture one past the recorded commit LSN, and with triggers the primary's `pg_current_wal_insert_lsn()` as the log read ran. Departing from the plan, `live.seal()` doesn't record `pg_current_wal_lsn()`: a deferred trigger runs before its own commit record is written, so a standby past that position may not show the commit; the insert position read after the commit is visible is sound, and costs nothing, since it's a column of the read the hub makes anyway. A round of re-runs reads from a replica past the furthest `lsn` among its commits and past the hub's floor, waiting up to `Live::replica_wait` (20 ms) and otherwise reading the primary (`Live::rerun_on_replicas(false)` turns it off); the floor keeps reads monotonic, measured again from the primary's insert position after anything read the primary (a first run, a fallback) and, with several replicas, raised to the chosen replica's position after its round; groups that join mid-round run on the primary. `LiveStats::replica_reruns` counts them. `db::testing`'s Postgres cluster (`tests/support/postgres.rs`) starts a standby with `pg_basebackup -R -X stream` and pauses and resumes replay. Tests: `leptoskit_db/tests/replicas.rs` (routing, a paused standby never chosen, commit positions in both capture modes); `leptoskit_rpc/tests/relay.rs` against a real `nats-server` (skipped without one unless `LEPTOSKIT_REQUIRE_NATS=1`; devenv has it): two servers on SQLite and on Postgres with a mutation on one reaching a subscriber on the other, 200 concurrent commits on the stream once each in order and chained, a publisher killed mid-flight replaced within its lease with the chain intact, a consumer catching up from the stream after NATS restarts with no database fill, no publisher, an oversize commit and a purged stream each filled from the database, channels and presence across servers, and re-runs that never read a paused standby; huddle's `tests/roles.rs`. **Numbers** (`examples/huddle/readme.md`, M4 Mac mini, everything on one machine, 20 messages a second for 30 s, every update delivered, no errors): one server on SQLite, 500 clients across 10 channels p50 4.8 ms / p95 24.8 ms / p99 40.8 ms and 5,000 across 50 p50 3.1 / p95 20.4 / p99 110.8 ms; one on Postgres 7.2 / 16.4 / 23.3 ms and 5.9 / 11.9 / 95.8 ms; two on Postgres through NATS (one with every role, one `web,live`, clients and messages split between them) 5.9 / 8.9 / 13.8 ms and 5.7 / 23.1 / 41.4 ms. The single hub needed no further fix for these: p99 is far below a second, and the earlier 1.16 s median was the per-connection feeds. The tail varies between runs (a second SQLite run's p99 was 220 ms; the worst single update 0.25 s on Postgres and 1.25 s on SQLite), and wasn't profiled further. Not built: sharding gateways by query hash (§9.2), and Redis Streams as a relay.

**L6 reach, 1 November 2026.** Streams (§3.5) are built: `#[rpc::stream]` (and `(resumable)`) is `Kind::Stream`, a `StreamOperation` rather than an `Operation`, since its body returns `impl Stream` and owns its `QueryCx` while it runs; `#[api]` publishes it as `ApiKind::Stream`, in AsyncAPI as a `receive` operation whose message is one item, and left out of OpenAPI and the HTTP SDK generators. The session holds each stream as a task with a semaphore of credits: `drive` takes a permit before it polls the body, so the body is never read past what the client granted, and the frames go through the session (the only writer) on a channel the credits bound. A stream ends with `end { reason, err? }`; an identity change or a drain interrupts every open stream, and the client re-opens a resumable one with `args.cursor` set to the last item's `Resumable::cursor`. Pages (§3.6) are an ordinary live query with a `PageArgs { after, until, limit }` argument returning `Page<T> { items, end, has_more }`, read with `rpc::paginate` by a unique ordered column; the cursor is that column's JSON. The convention, not a server-side helper with state, was chosen so a page is just another subscription: its read set covers exactly its range, it shares re-runs, and it resumes like any query. Pinning is the client's: `load_more` rewrites the last page's param to `until = end` (re-subscribing it) before fetching the next, so the first page's end is recorded in its own arguments. A page result patches its `items` (`Patch { field, set }`), only for objects shaped exactly like a page, so older clients never see one. The offline queue keeps calls in dispatch order (one queue for mutations and actions), resends mutations with their keys and unsent actions on each connection, and with `offline_queue(true)` writes the queue to `localStorage` or the store plugin through one ordered writer, only after what was kept is restored; an action is dropped from the device before it is sent. Actions still run in their own task on the server, so a replayed action may commit after a mutation queued behind it. `reauthenticate()` sets a flag: an open connection sends `auth` when the credentials hold a bearer token, and otherwise reconnects without resuming (a cookie can't change on an open socket); a handshake with the flag set also skips resume. `leptoskit_auth` calls it from `TokenKeeper::hold`/`clear` and `Cookie::keep`/`forget`. `mutation_scope::<M>()` goes over the connection while it is live (or the offline queue is on) and over HTTP otherwise; `use_live_mutation` is now the same thing. Known gaps: a pinned page grows without bound as rows land in its range (Convex splits such pages); generated SDKs don't speak streams; the guest store plugin caps one value at 64 KiB, so a very long queue isn't kept. Huddle now re-authenticates after sign-in and sign-out, and disposes its signed-in child owner on sign-out.

Companion plans: [database-plan.md](database-plan.md) (tables, policies, migrations, packs) and [auth-plan.md](auth-plan.md) (sessions, native tokens, the unified client).

## The problem

Several people use one app at once: a chat, a shared board, an order queue, a multiplayer form. When one of them changes something, everyone else should see it within a frame or two, on the web, on iOS, on Android and on the desktop, without anyone writing socket code, cache-invalidation code or "which clients care about this row?" code.

Today a leptoskit app can't do this without help:

- `leptoskit_query` refetches on focus, on an interval, or when the app invalidates by hand. Other people's writes are invisible until then.
- `QueryClient::broadcast` keeps the browser tabs on one machine in step, but not different machines.
- `leptoskit_plugin_websocket` gives a guest a raw socket, but no protocol, no auth, and no link to the database.
- Native guests contain no server functions, so anything real-time has to work over a protocol both the browser and a guest speak. The unified auth client set this precedent (auth-plan, sixth layer).

## The goal, in one example

```rust
// Shared crate: compiled into the server (bodies) and every client (stubs).

/// The latest messages in a channel, oldest first.
#[rpc::query]
pub async fn messages(cx: &rpc::QueryCx, channel: Id<Channel>, limit: u32) -> rpc::Result<Vec<PublicMessage>> {
	let rows = Message::query()
		.filter(|m| m.channel_id.eq(channel))
		.order_by(|m| m.created_at.desc())
		.limit(u64::from(limit.min(200)))
		.all(cx.db())
		.await?;
	Ok(rows.into_iter().rev().map(Into::into).collect())
}

/// Posts a message as the signed-in person.
#[rpc::mutation]
pub async fn send_message(cx: &rpc::MutationCx, channel: Id<Channel>, body: MessageBody) -> rpc::Result<Id<Message>> {
	let author = cx.require_user::<User>()?;
	let message = NewMessage { channel_id: channel, author_id: author, body }.insert(cx.tx()).await?;
	Ok(message.id)
}
```

```rust
// Any client: web, iOS, Android, macOS, Windows, Linux, HarmonyOS.
#[component]
fn ChannelView(channel: Signal<Id<Channel>>) -> impl IntoView {
	let messages = use_live(move || Messages { channel: channel.get(), limit: 50 });
	let send = use_live_mutation::<SendMessage>();

	view! {
		<View class="flex-1">
			<LiveList query=messages key=|m| m.id let:message>
				<Text>{message.body.to_string()}</Text>
			</LiveList>
			<Composer on_submit=move |body| send.dispatch(SendMessage { channel: channel.get(), body }) />
		</View>
	}
}
```

That's the whole app-side cost:

- **No subscriptions declared.** The server records which rows each query read.
- **No channels named.** The server works out which subscriptions a write affects.
- **No invalidation calls.** Every open `messages` query for that channel, on every device, updates on its own.
- **Read-your-writes.** `send.dispatch(…)` resolves only after the sender's own `messages` result includes the new message.
- **Access policies hold.** Every refresh runs under the subscriber's own policies, so nobody ever receives a row they couldn't fetch.

## What we learned from Convex and Serverpod

### Convex

| Convex                                                                                                                                                                | What we take                                                                                                                                                                                          | What we change                                                                                                                                     |
| --------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Query functions are deterministic and read-only**, so the server can re-run them whenever their inputs change                                                       | `#[rpc::query]` gets a read-only, tracked database handle. Writing from a query fails to compile                                                                                                      | Rust can't sandbox `reqwest`, so determinism is a documented rule, not a guarantee. Debug builds warn when a query takes longer than a budget      |
| **Read sets are index ranges** recorded while the query runs                                                                                                          | Read sets are **typed predicates**: table, `Expr` filter (including the policy condition), order and limit. They come free from the query builder, because every `Query<T>` is already an `Expr` tree | Convex owns its storage engine. We sit on Postgres and SQLite, so we capture writes with triggers that migrations generate (§4)                    |
| **One transaction log and a single committer** give every commit a global timestamp                                                                                   | A global, commit-ordered **version** for every transaction that touches a live table (§4.3)                                                                                                           | Postgres commits in parallel; only the last step of a commit, which allocates its version, is serialized                                           |
| **The subscription manager** walks the log once for all subscriptions and re-runs only the queries whose read sets overlap                                            | The `LiveHub` (§5.2): an index from (table, column, value) to subscriptions, so the cost of a write grows with the subscriptions it touches, not with all of them                                     | Matching can answer "unknown" (for example `ILIKE` under a collation). Unknown means re-run, so precision only ever costs speed, never correctness |
| **All of a client's queries sit at one timestamp**, so the UI never mixes states                                                                                      | Every update the server sends names a version, and the client applies all the updates for that version in one reactive batch                                                                          | None                                                                                                                                               |
| **A mutation's promise resolves only once the query results include it**; optimistic updates are dropped at that point                                                | The same: a `MutationResponse` carries its commit version, and the client waits for a transition at that version or later                                                                             | Optimistic updates reuse `leptoskit_query`'s existing `on_mutate`, rather than a separate API                                                      |
| **Mutations are serializable and retried automatically** on conflict                                                                                                  | `#[rpc::mutation]` runs at `SERIALIZABLE` on Postgres (SQLite already is) and retries on `40001`                                                                                                      | Side effects go through `cx.after_commit(…)` or `cx.enqueue(…)`, never inline                                                                      |
| **Actions** for side effects; the **scheduler** (`runAfter`) and **crons**, written in the mutation's own transaction; the **workflow** component for multi-step work | `#[rpc::action]`; `#[rpc::job]` with `cx.enqueue(…)`, `.after(…)` and `#[rpc::cron]`; checkpointed `cx.step(…)` (§3.7)                                                                                | Job state is a live table, so a client watches a job the way it watches any query                                                                  |
| **Paginated queries** keep page boundaries stable while the data moves                                                                                                | `use_live_paginated`, which pins each page by its end cursor (phase L6)                                                                                                                               | None                                                                                                                                               |
| **One WebSocket per client**, carrying queries, mutations and auth. `Authenticate` replaces the identity without dropping subscriptions                               | One socket per client, the same message set, and re-authentication in place when a native access token renews                                                                                         | We add a read-only SSE fallback (§6.4)                                                                                                             |
| **Self-hosting** is a single node with SQLite or Postgres underneath; horizontal scale is Convex Cloud's business                                                     | A single binary and SQLite is our smallest deployment too                                                                                                                                             | **Scale-out is ours to design:** stateless gateways, fan-out over Postgres `LISTEN/NOTIFY`, then logical replication and a relay (§9)              |

### Serverpod

| Serverpod                                                                                                                                                        | What we take                                                                                                                                                                                        |
| ---------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Streaming methods:** an endpoint method that takes or returns a `Stream`, multiplexed over one WebSocket. Every stream closes when the socket drops            | `#[rpc::stream]` for open-ended server output (progress, log tails, AI token streams) over the same socket (§3.5). Our streams resume or end with a typed reason, rather than closing silently      |
| **Server events:** `session.messages.postMessage(channel, msg)` and `createStream(channel)`. Global messages go through Redis; without Redis, sending one throws | `#[rpc::channel]` for ephemeral, non-database messages such as typing indicators, cursors and presence (§3.4). Cross-server delivery uses Postgres `NOTIFY` by default, so **no Redis is required** |
| **Generated client code from the server definitions** (Dart)                                                                                                     | Live operations are kinds of `#[api]` operation in the existing RPC spec, so `leptoskit rpc generate` writes live methods into every SDK it already makes (§8)                                      |
| **Future calls:** server work scheduled to run later                                                                                                             | `#[rpc::job]` with `.after(…)` and `.at(…)`, enqueued in the caller's transaction (§3.7)                                                                                                            |
| **Modules** that ship endpoints and tables                                                                                                                       | Our schema packs: `live` and `rpc` are packs (§4.2), and any pack can ship operations                                                                                                               |

**The synthesis:**

- **Convex's model for state,** which is almost everything: reactive queries over the database, transactional mutations, and consistency.
- **Serverpod's model for traffic that isn't state:** channels, presence and streams.
- **Durable jobs for slow work,** whose progress is itself state, so it reaches the screen the same way.

All of it runs over one connection, one auth handshake and one client, in the crate the RPC already lives in.

## 1. Architecture

```text
web (browser WebSocket) ─┐                 ┌───────────────── server instance (axum) ───────────────────┐
iOS/Android/desktop      │   one socket    │  /live  ──▶ Gateway (per connection: identity, query set,  │
(guest → websocket ──────┼────────────────▶│              mutation receipts, channel joins, streams)    │
 plugin)                 │                 │                    │                       ▲                 │
                         │                 │          run query │ (tracked LiveDb)      │ transitions     │
                         │                 │                    ▼                       │                 │
                         │                 │   Operation registry (#[rpc::query/mutation/job/…])      │
                         │                 │                    │                       │                 │
                         │                 │                    ▼                       │                 │
                         │                 │   LiveHub: subscription index · matcher · result cache     │
                         │                 │            · re-run at one snapshot · diff                  │
                         │                 └──────────────┬──────────────────────────────▲──────────────┘
                         │                    writes (Tx) │                    LISTEN    │ read changes
                         │                                ▼                              │ since version
                         │              ┌──────────────── Postgres / SQLite ─────────────┴───────────┐
                         │              │ app tables ──(row triggers)──▶ live.changes               │
                         │              │ commit: version allocated ──▶ live.commits + NOTIFY         │
                         │              └────────────────────────────────────────────────────────────┘
```

Three ideas carry the design:

1. **Invalidate, don't replicate.** The plan already had this rule. A change is never sent to clients. It only decides which queries to re-run, and the re-run (policy-filtered, for that subscriber) produces what is sent. Hidden rows can't leak, because no row reaches a client except through a query it was allowed to run.
2. **The database is the source of truth for change.** Triggers that migrations generate capture every write to a live table, from the ORM, raw SQL, `psql`, or another service, in the writing transaction. Nothing depends on the writer going through our process.
3. **One version line.** Every commit to a live table gets the next version. Clients, gateways and the hub all speak in versions: a resume point, a consistency label, and a read-your-writes token.
4. **Slow work leaves the request, but stays visible.** A mutation enqueues a job in its own transaction and returns at once. Workers update the job's row and the app's own rows as they go, and those are live tables, so every screen follows along (§3.7).

## 2. Crates and features

There is no new crate. The RPC already owns "operations the app exposes": the macros, the spec and the generated clients. Live queries, mutations, streams, channels and jobs are more kinds of operation, so they go there too. The database side splits off only where it is schema work.

| Crate                   | What changes                                                                                                                                                                                                                                                                                                                                                                   |
| ----------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `leptoskit_rpc`         | New features: `client` (wasm-safe: operation traits, protocol types, hooks), `web` (browser WebSocket and `EventSource`), `guest` (`leptoskit_plugin_websocket`), `server` (the gateway, the registry, the axum routes, the job workers), and `db` (live queries, receipts, jobs and the inbox over `leptoskit_db`; implies `server`). Today's `ssr` feature keeps its meaning |
| `leptoskit_rpc_macros`  | `#[rpc::query]`, `#[rpc::mutation]`, `#[rpc::action]`, `#[rpc::stream]`, `#[rpc::channel]`, `#[rpc::job]` and `#[rpc::cron]`, next to `#[api]`                                                                                                                                                                                                                                 |
| `leptoskit_rpc_codegen` | A `kind` on every operation in `ApiSpec`; AsyncAPI output; live methods and the protocol runtime in every generated client (§8)                                                                                                                                                                                                                                                |
| `leptoskit_db`          | Change capture, which works without the RPC: read/write connection split, in-memory `Expr` evaluation, read-set recording, subquery expressions, the `live` table option, and a change feed (`db.changes().since(version)`) that the RPC's hub reads (§4)                                                                                                                      |
| `leptoskit_db_schema`   | Triggers, trigger functions, sequences and constraint triggers as schema objects the diff understands                                                                                                                                                                                                                                                                          |
| `leptoskit_query`       | "Externally driven" queries: cache entries whose data the client pushes rather than fetches (§6.2)                                                                                                                                                                                                                                                                             |
| `leptoskit_auth`        | Implements the RPC's `Identify` trait, which resolves a bearer token or cookie to a `Context`, behind an `rpc` feature. The RPC defines the trait, so the dependency points one way                                                                                                                                                                                            |
| `leptoskit_cli`         | `[rpc.live]` and `[rpc.jobs]` config, `lk rpc check`, `lk jobs`, `lk generate deploy` (§10), and live and job pages in `lk dev`                                                                                                                                                                                                                                                |

**Guest builds stay small.** A native guest enables only `client` and `guest`, so it carries the operation structs, the protocol and the socket client, and none of the server, database or job code. The facade's existing features grow the same way as for auth:

```toml
rpc = ["dep:leptoskit_rpc", "leptoskit_rpc/client"]
# `web`, `guest` and `server` turn on leptoskit_rpc?/web, ?/guest and ?/server; `db-server` turns on leptoskit_rpc?/db.
```

## 3. The API

### 3.1 Live queries

```rust
#[rpc::query]
pub async fn channels(cx: &rpc::QueryCx) -> rpc::Result<Vec<ChannelCard>> {
	// A shape (database-plan §5.2): nested reads are tracked level by level.
	Channel::query().order_by(|c| c.name.asc()).shape::<ChannelCard>().all(cx.db()).await
}
```

**What the macro generates.** It follows leptos `#[server]` conventions:

- **An arguments struct** in PascalCase, `Channels {}` or `Messages { channel, limit }`, with serde and `JsonSchema`, compiled on every target.
- **`impl rpc::Operation for Messages`, with `const KIND: Kind = Kind::Query`**, with `const NAME: &str = "messages"` (or `#[rpc::query(name = "chat.messages")]`) and `type Output`.
- **On the server**, the body as `Operation::run`, registered through `inventory`, like `#[api]`.
- **On a client**, no body at all. A guest build carries only the struct and the name.

**`QueryCx`:**

- `cx.db()` is a **`&LiveDb`**: policy-scoped to the subscriber, read-only, and recording what it reads.
- `cx.user_id::<U>()`, `cx.signed_in()` and `cx.get::<G>()` read the same globals as policies.

**Read-only by type (L0).** `leptoskit_db`'s `Connection` splits into `Read` and `Write: Read`. Every write method (`insert`, `save`, `update().run`, `Query::delete`) takes `&impl Write`. `LiveDb` implements only `Read`, so a write inside a query is a compile error.

**Live tables only, by type.** A query on `LiveDb` needs `T: LiveTable`, which only `#[db::table(live)]` implements. Reading a table that isn't captured fails to compile, with a hint (§4.1): you can't write a live query that silently never updates.

**What a read set records.** Each read appends one entry to the query's read set:

| Read                                        | Recorded as                                                                                                                                             |
| ------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `Query<T>::all/first/one/count/exists`      | `(T, filter ∧ policy(select) ∧ soft-delete default, order, limit, result keys)`                                                                         |
| `T::get(id)`, `find`, link accessors        | `(T, key = id)`: a point read                                                                                                                           |
| Shapes                                      | One entry per level. Children read as `fk IN (parent keys)`                                                                                             |
| Policy subqueries (`cx.count`, `cx.exists`) | Their own entries, so revoking a membership re-runs the queries it gated                                                                                |
| `db.sql(…)` on `LiveDb`                     | Must declare `.reads::<Message>().reads::<User>()`: table-level entries. Undeclared raw SQL fails at run time in debug builds and is refused in release |
| Views                                       | `#[db::view(live, reads(Post, User))]`: table-level entries                                                                                             |

### 3.2 Mutations

```rust
#[rpc::mutation]
pub async fn rename_channel(cx: &rpc::MutationCx, channel: Id<Channel>, name: ChannelName) -> rpc::Result<()> {
	let mut row = Channel::get(cx.tx(), channel).await?;       // policies apply: NotFound if hidden
	row.name = name;
	row.save(cx.tx()).await?;                                  // Forbidden if the update policy refuses
	cx.publish(ChannelEvents::of(channel), ChannelEvent::Renamed);  // ephemeral, sent only after commit
	Ok(())
}
```

**The transaction:**

- **Isolation and retries.** Each mutation runs in one transaction at `SERIALIZABLE` on Postgres (SQLite is serializable already). Serialization failures (`40001`) and deadlocks (`40P01`) re-run the whole body, by default up to 5 times with jittered backoff (`#[rpc::mutation(retries = 0)]` turns this off). That's why a mutation body must not call the outside world: anything that leaves the transaction goes through:
  - `cx.after_commit(async move { … })`, which runs once, after commit, best effort;
  - `cx.enqueue(SomeJob { … })`, which is durable and transactional: the job exists only if the mutation commits (§3.7);
  - `cx.publish(channel, message)`, which is ephemeral and sent after commit (§3.4).
- **Exactly-once effects.** Every call carries a client-generated request ID. The mutation writes a receipt `(session, request_id) → (version, result)` in its own transaction. A retry after a dropped connection finds the receipt and returns the stored result instead of running twice. This matters on mobile networks, where "did my send go through?" is the commonest real-time bug. Receipts expire with the change log's retention.
- **The result.** It carries the commit version. The client resolves the mutation only once its own query set has reached that version (read-your-writes, §6.2).

**Calling it elsewhere:**

- **Server code** calls a mutation directly, as `SendMessage { … }.run(&cx)` or `rpc::call(&db, SendMessage { … })`. That's the same transaction, receipts and retries, with no socket.
- **A leptos server function** can call it too, so web-only apps can keep `#[server]` for form posts and still get live updates.

### 3.3 Actions

```rust
#[rpc::action]
pub async fn summarize_channel(cx: &rpc::ActionCx, channel: Id<Channel>) -> rpc::Result<String> {
	let messages = cx.query(Messages { channel, limit: 200 }).await?;  // runs as the caller
	let summary = my_llm::summarize(&messages).await?;                 // side effects are fine here
	cx.mutate(PinSummary { channel, summary: summary.clone() }).await?;
	Ok(summary)
}
```

Actions are not transactional, not retried, and can call anything. They reach data only through queries and mutations, so policies and receipts still apply. This is Convex's split, and it keeps mutations safe to retry.

### 3.4 Channels and presence (ephemeral, Serverpod-style)

Some state should never touch the database: "Ada is typing", cursor positions, who's looking at this document, the drag preview on a board. A channel is a typed pub/sub topic with an authorization rule:

```rust
/// Typing indicators and presence in one chat channel.
#[rpc::channel]
pub struct ChannelRoom {
	pub channel: Id<Channel>,
}

impl rpc::Channel for ChannelRoom {
	type Message = RoomSignal;          // enum { Typing { until_ms: u32 }, StoppedTyping }
	type Presence = Viewer;             // what each member shares: name, avatar, focus

	/// Who may join. Runs once on join; the hub re-checks it when the identity changes.
	async fn authorize(&self, cx: &rpc::QueryCx) -> rpc::Result<()> {
		Membership::query().filter(|m| m.channel_id.eq(self.channel)).exists(cx.db()).await?
			.then_some(()).ok_or(rpc::Error::Forbidden)
	}
}
```

```rust
// Client
let room = use_channel(move || ChannelRoom { channel: channel.get() });
room.set_presence(Viewer { name, focus: Focus::Composer });  // throttled, last-write-wins
room.send(RoomSignal::Typing { until_ms: 3_000 });
let typing = room.messages();          // Signal of recent messages
let here = room.presence();            // Signal<Vec<(ConnectionId, Viewer)>>, with join and leave
```

**Delivery guarantees:**

- **At most once, in order per sender.** Messages are never stored.
- **Presence** is a set with heartbeats: a member drops out after two missed heartbeats (default 30 s each), or immediately when their socket closes cleanly.
- **Bounds:** a message is at most 4 KiB by default (it must fit a Postgres `NOTIFY` payload), with rate limits per connection and per channel.

**Across instances,** channel traffic rides the same fan-out as database changes (§9): `NOTIFY` on Postgres by default, or the relay in phase L7.

### 3.5 Streams

```rust
#[rpc::stream]
pub async fn import_progress(cx: &rpc::QueryCx, job: Id<ImportJob>) -> rpc::Result<impl Stream<Item = Progress>> { … }
```

A stream is server output over time that isn't a query over rows: a long job's progress, a model's tokens, a log tail.

- **Flow control:** each stream is multiplexed on the socket with credits, so a slow phone can't make the server buffer without limit.
- **Ending:** a stream ends with `Done`, `Failed(error)` or `Interrupted`. On reconnect the client re-opens an interrupted stream only if it is marked `#[rpc::stream(resumable)]` and takes a cursor.
- **When to prefer a query.** If the state could be a row, such as `import_jobs.progress`, make it a live query instead, which survives reconnects and server restarts for free. The docs will say so first.

### 3.6 Client hooks

These live in `leptoskit_rpc::client` and are the same on every platform, like the auth client:

| Hook                                                 | Returns                                                                                                                                          |
| ---------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------ |
| `use_live(move \|\| Q { … })`                        | `Live<Q::Output>`: `Loading`, `Ready(T)` or `Failed(rpc::Error)`, plus `.version()`. Re-subscribes when its arguments change                     |
| `use_live_paginated(move \|\| Q { … }, page_size)`   | Pages with `.load_more()`; boundaries stay put while rows move (L6)                                                                              |
| `use_live_mutation::<M>()`                           | A `leptoskit_query` mutation: `.dispatch(args)`, `.pending()`, `.result()`, `.with_on_mutate(…)` for optimistic updates                          |
| `use_action::<A>()`, `use_stream(move \|\| S { … })` | The same shape for actions and streams                                                                                                           |
| `use_channel(move \|\| C { … })`                     | `.send`, `.messages()`, `.set_presence`, `.presence()`                                                                                           |
| `use_live_status()`                                  | `Connecting`, `Live { version }`, `Reconnecting { attempt, next_in }` or `Offline`, for a connection banner                                      |
| `use_job(job_id)`                                    | `JobState`: `Queued`, `Running { progress }`, `Retrying { attempt, next_at, error }`, `Succeeded(output)`, `Failed(error)` or `Cancelled` (§3.7) |
| `use_inbox()`                                        | The signed-in person's notices, newest first, with `.unread()` and `.mark_read(id)` (§3.8)                                                       |
| `rpc::configure(RpcConfig { origin, … })`            | One setup call, shared with the auth client's `ClientConfig` so the origin is stated once                                                        |

`LiveList` and `LiveFor` are thin wrappers over `For` and `VirtualList` that key rows by `Id` and animate inserts and removals. The virtual list already exists in `leptoskit_virtual`.

### 3.7 Background work: jobs

**The problem.** Some work is too slow, or too unreliable, to do while someone waits:

- sending an email or a text message;
- submitting a blockchain payment, which takes seconds to minutes to confirm and can fail;
- calling a slow or rate-limited outside API: an AI model, a payment provider, a shipping service;
- resizing uploads or importing a spreadsheet.

The person should get an answer at once ("sending…"), and then hear what happened, on whichever device they're using, even if they've moved to another screen. If the server restarts halfway through, the work must neither vanish nor run twice.

**The shape of the answer:**

1. A mutation records the intent as a row, such as a `payments` row with status `pending`, and enqueues a job **in the same transaction**. Both happen or neither does.
2. The client shows the pending row at once. It can show it even before the mutation returns, through an optimistic update.
3. A worker runs the job: retried with backoff, checkpointed step by step, and never lost when a server restarts.
4. The job updates the row (`paid`, or `failed` with a reason) through a mutation. The row is in a live table, so every screen that shows it updates. Anyone not looking at it gets a notice in their inbox (§3.8).

**Example 1: an email, and "Email sent" on the sender's screen.**

```rust
#[rpc::mutation]
pub async fn invite_member(cx: &rpc::MutationCx, channel: Id<Channel>, email: Email) -> rpc::Result<Id<Invite>> {
	let invite = NewInvite { channel_id: channel, email, status: InviteStatus::Sending }.insert(cx.tx()).await?;
	cx.enqueue(SendInvite { invite: invite.id });   // committed with the invite, or not at all
	Ok(invite.id)
}

/// Sends the invitation. Runs on a worker; nobody waits for it.
#[rpc::job(queue = "mail", retries = 8)]
pub async fn send_invite(cx: &rpc::JobCx, invite: Id<Invite>) -> rpc::Result<()> {
	let invite = Invite::get(cx.db(), invite).await?;
	cx.mailer().send(InviteEmail::from(&invite).idempotency_key(cx.key())).await?;
	cx.mutate(MarkInviteSent { invite: invite.id }).await?;   // the sender's list now says "Email sent"
	Ok(())
}
```

"Email _received_" needs the mail provider to report delivery. That's a webhook route that calls a mutation such as `MarkInviteDelivered`, and the same live update follows.

**Example 2: a blockchain payment, shown at once and settled later.**

```rust
#[rpc::mutation]
pub async fn send_payment(cx: &rpc::MutationCx, to: Id<User>, amount: Lamports, signed: SignedTransaction) -> rpc::Result<Id<Payment>> {
	let payment = NewPayment { to, amount, signature: signed.signature(), status: PaymentStatus::Pending }.insert(cx.tx()).await?;
	cx.enqueue(SettlePayment { payment: payment.id, signed }).key(payment.id);   // one settlement job per payment
	Ok(payment.id)
}

#[rpc::job(queue = "chain", serialize_by = payment, retries = 20, backoff = exponential(2s, 5m), deadline = 15m,
           on_failure = payment_gave_up)]
pub async fn settle_payment(cx: &rpc::JobCx, payment: Id<Payment>, signed: SignedTransaction) -> rpc::Result<()> {
	// Resending the same signed bytes is safe: the signature is the network's idempotency key.
	cx.step("submit", async || chain::submit(&signed).await).await?;
	cx.mutate(SetPaymentStatus { payment, status: PaymentStatus::Submitted }).await?;
	cx.progress("Waiting for confirmation");

	match cx.step("confirm", async || chain::wait_finalized(signed.signature()).await).await? {
		Finality::Finalized => cx.mutate(MarkPaid { payment }).await?,
		Finality::Dropped(reason) => {
			cx.mutate(MarkPaymentFailed { payment, reason: reason.clone() }).await?;
			cx.notify(cx.owner(), Notice::payment_failed(payment, &reason)).await?;   // "Fee too low · Retry"
		}
	}
	Ok(())
}

/// Runs once, after the last retry or the deadline: the payment must not stay "pending" forever.
#[rpc::mutation]
async fn payment_gave_up(cx: &rpc::MutationCx, job: rpc::Failed<SettlePayment>) -> rpc::Result<()> { … }
```

What each person sees:

```text
tap "Send 0.5 SOL"
 ├─ on_mutate: an optimistic card, "Sending 0.5 SOL to Ada…"
 ├─ the mutation commits: a real `pending` row, with the job queued in the same transaction
 │     (the card swaps optimistic for real without flickering: read-your-writes, §6.2)
 ├─ worker, step "submit":  status = submitted        → every device showing the payment updates
 ├─ worker, step "confirm": status = paid ✓           → "Payment to Ada confirmed" in the inbox
 │                    or:   status = failed + reason  → "Payment failed: fee too low · Retry"
 └─ Retry calls a mutation that builds a new transaction and enqueues a new job
```

**What a job guarantees, and what it asks of you:**

- **At least once.** A job runs until it succeeds, is cancelled, or exhausts its retries or deadline. A worker that crashes mid-job loses its lease, and another worker picks the job up.
- **Steps run once.** `cx.step(name, …)` saves its result in the job's row. A retried job skips completed steps and resumes at the first unfinished one. That's how a payment job avoids resubmitting after it already saw confirmation. A crash _during_ a step re-runs that step, so a step that reaches the outside world uses `cx.key()`, a stable key for the job and step. Email providers, payment providers and blockchains (through the signature) all accept one. Effects inside the database are exactly once already, because `cx.mutate(…)` uses receipts (§3.2).
- **Failures are typed.** A step returns `rpc::Error::Retry` (the default for network errors) or `rpc::Error::Permanent`, which stops retrying at once. `on_failure` names a mutation that runs once with the job's arguments and its last error, so the app's own row never stays `pending` forever.

**Options on `#[rpc::job]` and `cx.enqueue(…)`:**

| Option                                      | Meaning                                                                                                                                                   |
| ------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `queue = "chain"`                           | Which queue. Each queue has its own concurrency (`[rpc.jobs.queues.chain] concurrency = 2`), so a rate-limited outside API never sees more than you allow |
| `serialize_by = field`                      | At most one running job per value of that argument. Use it for a wallet's nonce order, or one user's import at a time                                     |
| `retries`, `backoff`, `deadline`, `timeout` | How many attempts, how far apart (exponential with jitter), how long overall, and how long one attempt may run                                            |
| `.after(duration)`, `.at(time)`             | Run later: reminders, trial expiry, "undo send" windows                                                                                                   |
| `.key(value)`                               | At most one queued or running job with this key, so enqueuing twice is safe                                                                               |
| `.priority(n)`                              | Lower runs first within a queue                                                                                                                           |
| `#[rpc::cron("*/5 * * * *")]`               | On a job with no arguments: one instance enqueues each tick, with the tick as the key, so a cluster runs it once                                          |

**Watching a job.** `cx.enqueue(…)` returns a `JobId<SettlePayment>`, which a mutation can return to the client:

- `use_job(job_id)` follows it live, because jobs are rows in a live pack table (§4.2).
- A job is visible to the person whose request enqueued it (`cx.owner()`), and to nobody else unless `#[rpc::job(visible_to = …)]` says so.
- `cx.progress("…")` and `cx.progress_fraction(0.4)` update the row, rate-limited to a few writes a second.
- `rpc::cancel(job_id)` asks a job to stop. A running job sees `cx.cancelled()` at its next step boundary.

**Model what people see in your own tables.** `use_job` suits generic progress bars and the developer pages. A payment's status belongs on the `payments` row, where policies, queries and history already apply, and where it outlives job retention.

**How jobs run:**

- **Storage:** rows in `rpc.jobs` in the app's own database, with no Redis or RabbitMQ (R14). Enqueueing is an insert in the caller's transaction, which is the transactional outbox pattern.
- **Claiming:** on Postgres, `UPDATE … WHERE id IN (SELECT … FOR UPDATE SKIP LOCKED LIMIT n) RETURNING *`, so workers on many instances never take the same job. On SQLite the single writer claims in process.
- **Waking:** an enqueue `NOTIFY`s the workers, so a job starts within milliseconds. A slow poll (`[rpc.jobs] poll_seconds = 5`) picks up scheduled jobs and anything a notification missed.
- **Leases:** a worker holds a job for `timeout` plus a margin, and extends the lease as it runs. An expired lease means the worker died, and the job is retried.
- **Where:** an in-process worker pool by default (`[rpc.jobs] workers = 4`), in the same binary as the web server. Shape C (§9.2) runs them as `--role worker`. On shutdown, workers stop claiming, then finish or release what they hold.
- **Retention:** finished jobs are kept for `[rpc.jobs] retention_days = 7`, then pruned. Failed jobs stay until retried or dismissed.
- **Operations:** `lk dev` shows queues, running jobs, failures and their errors at `/_leptoskit/jobs`, with a retry button. `lk jobs list|retry|cancel` does the same against any deployment.

**What we borrowed.** Convex's scheduler and its workflow component (steps); Serverpod's future calls; Oban for Elixir (Postgres-backed, `SKIP LOCKED`, unique jobs, cron); and Temporal and Inngest for memoized steps.

### 3.8 Telling people: the inbox

When slow work finishes, the person who started it may be on another screen or another device. They need a notice, not just an updated row somewhere.

```rust
cx.notify(user, Notice {
	title: "Payment failed".into(),
	body: "The network fee was too low.".into(),
	link: Some(route!("/payments/{payment}")),   // deep link, through the linking plugin on native
	action: Some(NoticeAction::new("Retry", RetryPayment { payment })),
	kind: NoticeKind::Error,
}).await?;
```

- **The inbox is a live table** in the `rpc` pack (`rpc.inbox`), written in the caller's transaction like any row. `use_inbox()` shows it, with an unread count, on every device the person has open, at once.
- **An action is an operation.** `NoticeAction` names a mutation and its arguments. Tapping it runs the mutation, under the person's own policies.
- **While the app is open in the background,** on the desktop and on Android, the client can raise a system notification for new inbox items through `leptoskit_plugin_notifications`. iOS suspends sockets in the background, so this isn't reliable there.
- **While the app is closed,** the notice waits in the inbox. Reaching a closed app needs remote push (APNs, FCM, Web Push), and `leptoskit_plugin_notifications` only shows local notifications today. Remote push is its own plan (R15). It will plug in as an inbox delivery channel (`trait Deliver`), alongside email through auth's `Mailer` for people who prefer it.

## 4. Schema and migrations

This section answers the question of how real time fits the database plan.

### 4.1 Opting a table in

```rust
#[db::table(name = "messages", public, live, index(channel_id, created_at))]
#[db::policy(select, |m, cx| cx.global(|me: &Viewer| cx.exists(Membership::query()
	.filter(|x| x.channel_id.eq(m.channel_id) & x.user_id.eq(me.0)))))]
#[db::policy(insert, |m, cx| cx.global(|me: &Viewer| m.author_id.eq(me.0) & cx.exists(Membership::query()
	.filter(|x| x.channel_id.eq(m.channel_id) & x.user_id.eq(me.0)))))]
pub struct Message {
	pub id: Id<Message>,
	#[db(on_delete = cascade)]
	pub channel_id: Id<Channel>,
	pub author_id: Id<User>,
	pub body: MessageBody,
	#[db(live = skip)]                 // never copied into the change log (big blobs, vectors)
	pub attachment_preview: Option<Vec<u8>>,
	#[db(flatten)]
	pub times: db::Timestamps,
}
```

**What `live` does:**

- **Generated code:** `impl LiveTable for Message`, and a `LIVE_COLUMNS` list: every column except `secret` and `live = skip`.
- **Migrations:** capture triggers on the table (§4.3). `lk db migration create` adds them when `live` appears, and drops them when it goes, like any other schema object.
- **No change to `Message`'s Rust API.** Writes from anywhere are captured.

**Why opt-in and not every table?** Capture costs every write a second insert (§4.5). Tables that are written hot and never watched, such as analytics events, rate limits and the auth pack's `challenges`, shouldn't pay it. The `LiveTable` bound makes forgetting `live` a compile error at the query that needed it, so opt-in costs no correctness.

**Pack tables** opt in the same way. The auth pack marks `sessions` live, so the hub can see revocations (§7).

### 4.2 The `live` and `rpc` schema packs

Two packs ship the system's own tables, so they land in the app's migration history like `auth.*` (database-plan §7), on the portable subset. `live` belongs to `leptoskit_db`, because change capture is useful without the RPC. `rpc` belongs to `leptoskit_rpc`.

| Table (Postgres `schema.table`; SQLite `schema_table`) | Columns                                                                                                                                                                                                             | Purpose                                                                                                      |
| ------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------ |
| `live.commits`                                         | `version bigint PK`, `txid xid8` (Postgres) or none (SQLite), `committed_at`, `actor` (from `leptoskit.user_id` when set), `origin` (operation name or `sql`)                                                       | One row per committing transaction that touched a live table: the version line                               |
| `live.changes`                                         | `txid` or `version`, `seq int`, `table_name text`, `op char(1)` (`I`/`U`/`D`), `key jsonb`, `old jsonb`, `new jsonb`, `changed text[]` (the column names, for updates only)                                         | One row per written row. `old` and `new` hold only `LIVE_COLUMNS`, so `secret` columns never leave the table |
| `rpc.receipts`                                         | `session_id`, `request_id uuid`, `version`, `result jsonb`, `at`; PK `(session_id, request_id)`                                                                                                                     | Exactly-once mutations (§3.2)                                                                                |
| `rpc.jobs` (live)                                      | `id`, `queue`, `name`, `args jsonb`, `key` (unique while queued or running), `owner`, `state`, `priority`, `run_at`, `attempt`, `lease_until`, `progress`, `steps jsonb`, `output jsonb`, `error jsonb`, timestamps | Durable background work (§3.7)                                                                               |
| `rpc.inbox` (live)                                     | `id`, `account`, `title`, `body`, `link`, `action jsonb`, `kind`, `read_at`, `created_at`                                                                                                                           | Notices for people (§3.8)                                                                                    |

**Retention.** `changes`, `commits` and `receipts` are kept for a resume window, `[rpc.live] retention_minutes = 15` by default. A pruning task on one instance (with a Postgres advisory lock) deletes older rows in batches. A client that reconnects after the window simply re-runs its queries from scratch, which is always correct. Resume is only an optimisation.

### 4.3 Capture: triggers, and a version at commit

**Postgres.** The pack's migration defines the functions once, and each live table gets a trigger that names its columns:

```sql
-- Once, in the live pack's migration. Helpers (live.key_of, live.project, live.changed),
-- live.change_seq and the live.pending (txid PRIMARY KEY) table are elided.
CREATE SEQUENCE live.version_seq;

CREATE FUNCTION live.capture() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE cols text[] := TG_ARGV;  -- the live columns, rendered by the migration
BEGIN
  INSERT INTO live.changes (txid, seq, table_name, op, key, old, new, changed)
  VALUES (pg_current_xact_id(), nextval('live.change_seq'), TG_TABLE_SCHEMA || '.' || TG_TABLE_NAME,
          left(TG_OP, 1), live.key_of(TG_TABLE_NAME, coalesce(NEW, OLD)),
          live.project(OLD, cols), live.project(NEW, cols), live.changed(OLD, NEW, cols));
  -- First write in this transaction: register for a version at commit.
  INSERT INTO live.pending (txid) VALUES (pg_current_xact_id()) ON CONFLICT DO NOTHING;
  RETURN NULL;
END $$;

-- Runs at COMMIT, once per transaction, after every other deferred check.
CREATE FUNCTION live.seal() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE v bigint;
BEGIN
  PERFORM pg_advisory_xact_lock(hashtext('leptoskit.live'));  -- serialize only this final step
  v := nextval('live.version_seq');
  INSERT INTO live.commits (version, txid, committed_at, actor, origin)
  VALUES (v, NEW.txid, clock_timestamp(),
          nullif(current_setting('leptoskit.user_id', true), '')::uuid,
          coalesce(nullif(current_setting('leptoskit.origin', true), ''), 'sql'));
  DELETE FROM live.pending WHERE txid = NEW.txid;
  PERFORM pg_notify('leptoskit_live', v::text);
  RETURN NULL;
END $$;

CREATE CONSTRAINT TRIGGER seal AFTER INSERT ON live.pending
  DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION live.seal();

-- Per live table, generated from #[db::table(live)]:
CREATE TRIGGER live_capture AFTER INSERT OR UPDATE OR DELETE ON public.messages
  FOR EACH ROW EXECUTE FUNCTION live.capture('id', 'channel_id', 'author_id', 'body', 'created_at', 'updated_at');
```

**Why versions are visible in commit order:**

- **The advisory lock.** It is transaction-scoped, so a transaction holds it from allocating its version until it commits. The next transaction can only allocate after that.
- **The consequence.** No version becomes visible after a higher one, so a reader that has seen `n` and reads `version > n` never misses a commit.
- **Holes are harmless.** A transaction that fails a later deferred check after taking its version rolls back, and leaves a hole in the sequence (its `NOTIFY` is discarded with it). Readers never wait for `n + 1` specifically, so a hole costs nothing.
- **The cost.** Only the tail of each commit is serialized: a sequence call and one insert. That's still far less contention than Convex's single committer, which serializes the whole commit check.
- **The ceiling.** A few thousand commits per second to live tables on ordinary managed Postgres, measured in L1. Above that, logical replication capture (L7) replaces the lock with the WAL's own commit order.

**Why `NOTIFY` and deferred constraint triggers:**

- **`NOTIFY`:** Postgres sends notifications only at commit, and in commit order, so listeners never hear about a rollback.
- **Deferred constraint triggers:** they are the standard way to run code at commit, and they're in every Postgres version we support.

**SQLite.** SQLite has one writer, so ordering is free:

- **Triggers.** There are three per live table (insert, update and delete), generated with explicit `json_object('id', NEW.id, …)` column lists, because SQLite has no generic row-to-JSON function. They insert into `live_changes` with `version = NULL`.
- **The version.** The `leptoskit_db` SQLite pool already funnels writes through one connection. Just before `COMMIT`, it runs `UPDATE live_changes SET version = ? WHERE version IS NULL` with the next version, inserts the `live_commits` row, and after commit wakes the in-process hub through a `tokio::sync::watch`.
- **Writes from another process,** such as the `sqlite3` shell, are captured by the triggers and get their version from the next commit through the pool. The docs say so.

### 4.4 What the schema crate and the diff learn

`leptoskit_db_schema` gains objects it can render, diff and snapshot, for both dialects:

| Object                                       | Postgres                                                                   | SQLite                                                   |
| -------------------------------------------- | -------------------------------------------------------------------------- | -------------------------------------------------------- |
| `Trigger { table, name, timing, events, … }` | `CREATE TRIGGER … EXECUTE FUNCTION live.capture(<columns>)`                | `CREATE TRIGGER … BEGIN INSERT INTO live_changes …; END` |
| `Function` (pack-owned only)                 | `CREATE OR REPLACE FUNCTION`, with the body checksummed into `schema.json` | n/a                                                      |
| `Sequence`                                   | `CREATE SEQUENCE`                                                          | n/a (a row in `live_meta`)                               |
| `ConstraintTrigger` (deferred)               | `CREATE CONSTRAINT TRIGGER … DEFERRABLE INITIALLY DEFERRED`                | n/a                                                      |

**Diff rules:**

- **Columns change:** adding or dropping a column on a live table, or a `secret`/`live = skip` change, re-renders that table's trigger as `DROP TRIGGER` then `CREATE TRIGGER` in the same migration.
- **Renaming a live table** renames its trigger.
- **Dropping `live`** drops the trigger. Old `changes` rows for the table age out with retention.

Triggers are one more object the diff understands. Like policies, views and generated columns, they're added to the list in database-plan §6.

`lk db check` learns three things:

- **Triggers match.** With `--live`, every live table's trigger exists and names the right columns. This catches a hand-edited database.
- **Indexes cover filters.** It warns when a live query's recorded read sets filter on a column with no index, from `lk dev`'s recordings (§11). Re-runs are frequent, so a sequential scan hurts more here than anywhere.
- **No wide blobs.** It warns when a live table has wide `bytea` or vector columns that aren't `live = skip`.

### 4.5 What capture costs

| Cost                                           | Size                                         | Mitigation                                                                                                              |
| ---------------------------------------------- | -------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------- |
| One extra row per written row                  | Roughly doubles the write I/O to live tables | Opt-in tables, `live = skip` columns, and `old` stored for updates only. In L7, logical replication removes it entirely |
| Serialized commit tail                         | Microseconds per commit, under the lock      | Only transactions that touched a live table take it                                                                     |
| WAL growth and pruning                         | Bounded by retention                         | Batched deletes. In L7, partition `changes` by hour and drop old partitions                                             |
| `pg_current_xact_id()` forces a transaction ID | Read-only transactions are unaffected        | Only writers fire triggers                                                                                              |

### 4.6 RLS mode

When `[db] policies = "rls"` (database-plan §4.9), transactions already `SET LOCAL leptoskit.user_id`. Capture records it as the commit's `actor`, which the dev tools show ("v4812 by ada@… via send_message"). Capture functions are `SECURITY DEFINER`, owned by the migration role, so RLS on app tables never blocks writing the change log.

## 5. The server

### 5.1 Wiring

```rust
// src/main.rs, after auth (auth-plan "Server wiring")
let rpc = rpc::Server::builder(&db)
	.auth(&auth)                      // identities, cookie and bearer, revocation (§7)
	.config(&config.rpc)
	.start()                          // runs the LISTEN loop, the hub, the job workers and pruning
	.await?;

let app = Router::new()
	.leptos_routes_with_context(&options, routes, auth.context(), App)
	.merge(auth.router())
	.merge(rpc.router())             // GET /live (WebSocket upgrade), GET /live/events (SSE), POST /live/rpc (one-shot calls)
	.layer(auth.layer());

leptoskit_axum::serve_router(addr, app).with_graceful_shutdown(rpc.drain()).await?;
```

```toml
# leptoskit.toml: [rpc] already holds the spec settings (title, servers, targets)
[rpc.live]
path = "/live"
transports = ["websocket", "sse"]
retention_minutes = 15
fanout = "postgres" # "postgres" (LISTEN/NOTIFY); "local" (one instance, the SQLite default); "relay" (L7)
max_connections_per_session = 10
max_queries_per_connection = 256
max_message_kib = 512 # below the guest plugin's 1 MiB default
heartbeat_seconds = 30
mutation_retries = 5

[rpc.jobs]
workers = 4
poll_seconds = 5
retention_days = 7

[rpc.jobs.queues.chain]
concurrency = 2
```

`[rpc.live]` and `[rpc.jobs]` follow the existing config conventions: snake_case keys, whole-number units, and secrets only through `_env` fields. `lk generate live` adds it, along with the server wiring above and `rpc::configure` on the client, idempotently, like `lk generate auth`.

### 5.2 The hub

The hub is one per instance. It keeps every subscription on that instance:

```text
Subscription { id, connection, query: (name, args), identity, read_set, result_cache_key, last_result_keys, version }
```

**The subscription index.** For each read-set entry, the hub picks the most selective way to index it:

- **An equality or `IN` on a column**, such as `channel_id = X` or `id IN (…)`, is indexed by `(table, column, value)`.
- **Anything else** (a range, `LIKE`, a constant) goes into a `(table)` bucket.

**Processing a version batch:**

1. **Read the changes.** Read `changes` for versions `(last, newest]` in one query.
2. **Find candidates.** For each change, look up `(table, column, old value)` and `(table, column, new value)` in the index, plus the table bucket.
3. **Match.** Run the matcher (§5.3) on each candidate entry's predicate against `old` and `new`, giving `NoMatch`, `Match` or `Unknown`.
   - Under a limit, a `Match` whose new sort key lands after the last row of a full result, with the old row not among `last_result_keys`, is skipped, because it can't enter the window.
   - Otherwise `Match` and `Unknown` mark the subscription dirty.
4. **Re-run.** Re-run each distinct dirty `result_cache_key` once (§5.4), at one snapshot per batch (§5.5).
5. **Diff and send.** Diff each new result against the old (§6.3) and send one `Transition { from, to, updates }` per connection.

### 5.3 The matcher

`Expr` is a small closed enum: `Compare`, `In`, `Null`, `Contains`, `Like`, `And`, `Or`, `Not`, `Constant`, plus `CompareColumn` and the `Count` subquery from the database stack. L0 adds `Expr::eval(&self, row: &JsonRow) -> Tri` (true, false or unknown) with SQL's three-valued logic.

**The rule that keeps this safe:** `NoMatch` only when certain. Anything dialect-sensitive returns `Unknown`, which means re-run:

- `ILIKE`/`LIKE`, which differ between Postgres and SQLite outside ASCII;
- collation-dependent text ordering;
- `Count` subqueries (`cx.count`, `cx.exists`), whose results the hub doesn't hold;
- timestamps that compare as `TEXT` on SQLite, unless they are in the fixed-width form, which they are by construction.

So a matcher bug can only cost a wasted re-run, never a missed update. A property test generates random `Expr`s and rows, and checks on both databases that `eval` never says `false` where SQL says `true` (§11).

### 5.4 Sharing work between subscribers

A chat channel with 2,000 viewers shouldn't run 2,000 identical queries per message. Each subscription's `result_cache_key` is the **rendered SQL plus bound parameters**, after policies are applied:

- **Viewers who would get the same SQL share one re-run and one result.** For example, policies that depend only on channel membership produce the same SQL for every member.
- **Viewers whose policies differ get different SQL, so they're never shared by mistake.** For example, a policy that compares `author_id` to the viewer.

This needs no annotation and can't leak, because sharing happens only when the database would literally run the same statement.

### 5.5 Consistency across a client's queries

**The guarantee:** within a connection, every update in one `Transition` reflects the same database state.

**How the hub gets it:**

- **One snapshot per batch.** The hub re-runs a batch's dirty queries inside one `REPEATABLE READ READ ONLY` transaction on Postgres (SQLite reads are snapshots already). It also reads `max(version)` from `live.commits` in that snapshot, giving `V'`.
- **Catching up inside the snapshot.** If `V'` is ahead of the batch's newest version, the hub matches the changes in between against every subscription on the affected connections. It re-runs any newly dirty ones in the same snapshot, then labels the transition `V'`. The snapshot is fixed, so this ends after at most one extra round.
- **The client's side.** The client applies a transition in one leptos `batch`, so no frame shows a channel's message count from one version and its messages from another.

### 5.6 Limits and backpressure

| Limit                           | Default      | On overflow                                                                                                                               |
| ------------------------------- | ------------ | ----------------------------------------------------------------------------------------------------------------------------------------- |
| Queries per connection          | 256          | The `sub` is refused with `limit`                                                                                                         |
| Outgoing bytes per connection   | 4 MiB        | Transitions coalesce: the connection skips intermediate versions and gets the latest. Real-time UIs want the newest state, not every step |
| Result size                     | 512 KiB      | The query fails with `too_large`, and the docs point to pagination. Diffs keep steady-state frames small                                  |
| Re-run concurrency per instance | `2 × CPU`    | Queued in version order. `live_rerun_queue_depth` is exported for autoscaling                                                             |
| Mutations per session           | Token bucket | `rate_limited` with `retry_after`, shaped like the auth limits                                                                            |

## 6. The protocol and the client

### 6.1 Frames (v1, JSON)

The WebSocket subprotocol is `leptoskit.live.v1.json`. A binary `postcard` encoding (`…v1.postcard`) is a later option, negotiated the same way.

```text
client → server                                   server → client
hello   { resume?: { version, queries } }         welcome     { connection, version, heartbeat_seconds }
auth    { token }                (bearer only)    transition  { from, to, updates: [ { query, ok | patch | err } ] }
sub     { query, name, args }                     done        { request, ok | err, version }      (mutation and action results)
unsub   { query }                                 auth_status { identity | expiring | refused }
mutate  { request, name, args }                   signal      { channel, from, message }
act     { request, name, args }                   presence    { channel, joined, left, state }
join    { channel, name, args, presence? }        item        { stream, value, cursor? } · end { stream, reason, err? }
signal  { channel, message } · leave { channel }  reconnect   { after_ms }     (drain, rebalancing)
stream  { stream, name, args, credits }           pong
credit  { stream, n } · cancel { stream }
ping
```

### 6.2 Inside the client

**One implementation, two seams.** This is the auth client's design (auth-plan, sixth layer), reused:

- **`Transport`** opens a duplex message stream:
  - a browser `WebSocket` through `web-sys` (`web`);
  - `leptoskit_plugin_websocket::guest::WebSocketClient` (`guest`);
  - an in-memory pipe to a real `Live` router (tests);
  - an SSE `EventSource` plus `POST`, read-mostly (`web`, fallback).
- **Credentials:**
  - **Web:** the `__Host-session` cookie rides the upgrade request. The server checks `Origin` on the upgrade (§7), because a WebSocket upgrade is a `GET` and passes the auth layer's same-site rule for safe methods.
  - **Native guests:** they can't set headers on the socket. The client sends `auth { token }` with the access token from the auth client's `TokenKeeper` right after `hello`, and again whenever the keeper renews it, so subscriptions are never dropped.

**Queries live in the existing `QueryClient`.** A live query is a `leptoskit_query` entry with `stale_time = ∞` and no fetcher of its own; the socket feeds it. In return, it gets for free:

- `.with_ssr()` hydration;
- `use_query`-style suspense;
- `QueryClient::broadcast` between tabs, so a second tab doesn't need its own socket for queries the first already holds. That's a later optimisation, through a `SharedWorker`.
- devtools, and `set_query_data` for optimistic updates.

**Read-your-writes and optimistic updates:**

- A mutation's `on_mutate` edits the cache optimistically, as `leptoskit_query` mutations already can.
- When `done { version: v }` arrives, the client holds the mutation open until a transition with `to ≥ v` has been applied. It then drops the optimistic layer and resolves.
- The UI never flickers back to the pre-mutation state between the response and the update.

**Reconnects:**

- **Backoff:** exponential with full jitter (250 ms up to 30 s).
- **Resume:** `hello { resume: { version, queries } }` re-subscribes everything. The server can answer with diffs from the cache if it has the old results, and otherwise re-runs at a fresh snapshot.
- **Pending mutations** are re-sent with the same `request` ID; receipts make that safe.
- **Offline queue:** mutations dispatched while offline queue in memory (and in L5 optionally in `leptoskit_plugin_store`) and send in order on reconnect.

**SSR:** `use_live` during server rendering runs the query in process, the way the auth client's `ssr` reads do, and streams the result with the HTML. The browser hydrates and then subscribes with `resume.version` set, so the first transition is usually empty.

### 6.3 Diffs

**When the server sends a diff.** When a result is a `Vec<T>` whose items have an `id` (`HasId` on the public type, generated for `Public*` types and shapes that select `id`), the server sends a keyed patch instead of the whole value:

```json
{
	"query": 7,
	"patch": {
		"remove": ["018f…"],
		"upsert": [{ "at": 49, "value": { "id": "0190…", "body": "hi" } }]
	}
}
```

**Everything else** is sent whole. Diffs keep a 2,000-message channel's update at about one message's size, and well under the guest plugin's 1 MiB frame limit.

### 6.4 SSE: when and why

**Recommendation: WebSocket is the protocol; SSE is a read-only fallback for the web.**

| Need                                                                                    | WebSocket                        | SSE                                                                                              |
| --------------------------------------------------------------------------------------- | -------------------------------- | ------------------------------------------------------------------------------------------------ |
| Subscriptions, mutations, channels and streams on one connection                        | Yes                              | Server-to-client only. Mutations and `sub`/`unsub` go as `POST /live/rpc` with the connection ID |
| Native guests                                                                           | Yes, through the existing plugin | No: the HTTP plugin doesn't stream responses                                                     |
| Networks that strip upgrades (some corporate proxies)                                   | Can fail                         | Plain HTTP; works through anything                                                               |
| Read-only consumers: dashboards, `curl`, generated TypeScript clients, server-to-server | Heavier                          | Native `EventSource`, automatic reconnect with `Last-Event-ID` (we put the version there)        |

The client falls back to SSE automatically after two failed upgrade attempts on the web, using the same frames. Supporting SSE costs one extra transport and one extra route, because the hub doesn't know which transport a connection uses.

## 7. Security

- **Policies everywhere:**
  - every query re-run uses `Db::with_context(subscriber's Context)`;
  - mutations use the caller's;
  - channels re-run `authorize` when the identity changes.
  - Nothing sends change rows to clients (§1).
- **Cross-site WebSocket hijacking.** The upgrade requires `Origin` to equal `[auth] origin`, or no `Origin` and a bearer `auth` frame for native clients.
- **Identity changes mid-connection:**
  - **Sign-in or sign-out** sends `auth`, or the web client reconnects. Every subscription is re-evaluated under the new identity, and results that are no longer visible become empty or `Forbidden`.
  - **Revocation:** `auth.sessions` is a live table. The hub watches the sessions of its connected sockets, and a revoked or expired session ends its connections with `auth_status { refused }` within one batch. `sign_out_everywhere` takes effect on open sockets immediately.
  - **Access-token expiry** for native clients: the server sends `auth_status { expiring }` a minute ahead, and refuses at expiry. The client renews through `TokenKeeper` and re-authenticates in place.
- **Resource abuse:** the per-connection limits in §5.6, per-IP connection caps, and frame size caps, enforced before parsing.
- **Code push skew.** A native app on an old code push release may call a query or mutation whose arguments have changed. See §8.5.

## 8. One API: live and the RPC

Live operations aren't a second API next to `leptoskit_rpc`. They are more kinds of operation in the same one, in the same crate. `#[api]` stays the single switch that publishes an operation to generated SDKs, and the second attribute says what kind it is:

```rust
#[api(tag = "chat")]
#[rpc::query]
pub async fn messages(cx: &rpc::QueryCx, channel: Id<Channel>, limit: u32) -> rpc::Result<Vec<PublicMessage>> { … }

#[api(tag = "chat", scope = Scope::ChatWrite)]   // IdP scopes (auth-plan) apply to every kind
#[rpc::mutation]
pub async fn send_message(cx: &rpc::MutationCx, channel: Id<Channel>, body: MessageBody) -> rpc::Result<Id<Message>> { … }

#[api]
#[server]                                        // today's kind, unchanged
pub async fn export_csv(range: DateRange) -> Result<String, ServerFnError> { … }
```

Without `#[api]`, an operation is still callable by the app's own clients, but it isn't in the published spec. That's what a bare `#[server]` means today.

### 8.1 The kinds

| Kind       | Declared with      | One-shot HTTP, `POST /api/<name>`                                                           | WebSocket, `/live`         | SSE, `GET /api/<name>/events`                  | In process (SSR, server code) |
| ---------- | ------------------ | ------------------------------------------------------------------------------------------- | -------------------------- | ---------------------------------------------- | ----------------------------- |
| `call`     | `#[server]`        | Yes, as today                                                                               | Yes, as a one-shot         | No                                             | Yes                           |
| `query`    | `#[rpc::query]`    | Yes: the value, plus its version in `X-Leptoskit-Version`                                   | Subscribe                  | Subscribe, with the version as `Last-Event-ID` | Yes                           |
| `mutation` | `#[rpc::mutation]` | Yes: `Idempotency-Key` becomes the receipt's request ID (§3.2), so a retried POST runs once | Yes, with read-your-writes | No                                             | Yes                           |
| `action`   | `#[rpc::action]`   | Yes                                                                                         | Yes                        | No                                             | Yes                           |
| `stream`   | `#[rpc::stream]`   | No                                                                                          | Yes, with credits          | Yes                                            | Yes                           |
| `channel`  | `#[rpc::channel]`  | Publish                                                                                     | Join, signal, presence     | Listen only                                    | Publish                       |
| `job`      | `#[rpc::job]`      | No: server code enqueues jobs, and a client starts one by calling a mutation                | Status through `use_job`   | Status                                         | Enqueue                       |

**What this unlocks:**

- **A one-shot query can hand over to a subscription without a gap.** A client that fetched over HTTP at version `v` subscribes from `v`, and the first transition carries only what changed since.
- **Streaming finds a home in `#[api]`.** Today `#[api]` refuses `StreamingText` server functions, because no generated client could call them. A `#[rpc::stream]` is publishable: SDKs read it over SSE or the socket.

### 8.2 One spec

**The spec gains a kind.** `ApiFunction` in `leptoskit_rpc_codegen` gets a `kind` field, and `SPEC_VERSION` moves. A `call` keeps its HTTP encoding fields. The other kinds always speak JSON, so their encodings are fixed. Channels also record their message and presence schemas. There is no separate `live.json`.

**Where the spec goes:**

- **`/openapi.json`** keeps describing what HTTP can call once: calls, queries, mutations, actions and channel publishes.
- **`/asyncapi.json`** is new. It describes the live side (subscriptions, streams, channels) as an AsyncAPI 3.0 document, the OpenAPI counterpart for event-driven APIs, so the same third-party tooling works.
- **`leptoskit rpc spec` and `leptoskit rpc generate`** read the one spec, as they do today.

### 8.3 Generated SDKs

Every generated client keeps today's one-shot methods and gains the live ones:

| Language   | One-shot                        | Live                                                                                         |
| ---------- | ------------------------------- | -------------------------------------------------------------------------------------------- |
| TypeScript | `await api.messages(args)`      | `api.messages.subscribe(args, onValue)` returns an unsubscribe; `api.channelRoom.join(args)` |
| Dart       | `await api.messages(...)`       | `api.messages.watch(...)` returns a `Stream`, ready for Flutter's `StreamBuilder`            |
| Python     | `api.messages(...)`             | `for value in api.messages.watch(...)`, with an `async for` variant                          |
| Go         | `api.Messages(ctx, args)`       | `api.Messages.Subscribe(ctx, args)` returns a channel                                        |
| Rust       | `api.messages(args).await`      | `api.messages_watch(args)` returns a `Stream`                                                |
| CLI        | `chat-cli messages --channel …` | `--watch` prints each new value as one line of JSON                                          |

**How a generated client speaks the live protocol:**

- **The runtime.** Each language gets one small protocol runtime (framing, reconnect with backoff, versions, diffs, receipts), emitted as a file next to the generated code. Clients stay self-contained, as they are today.
- **The transport.** A client uses the WebSocket where the language has one, and falls back to SSE for reads.
- **Auth.** Clients authenticate with a bearer token: a native session's access token, or an OAuth token from `leptoskit_idp`, whose typed scopes now gate subscriptions too.
- **A React adapter.** An optional `@scope/name/react` entry point adds `useQuery(api.messages, args)`, like Convex's hook.

### 8.4 Inside a leptoskit app

The app's own Rust client is the same idea, without code generation. The macros already emit the argument structs, so:

- **`use_live(...)`, `use_live_mutation::<M>()` and `call(Op { … }).await`** work for every kind, on the web, in native guests and during SSR.
- **Native guests reach everything.** The guide says today that native guests contain no server-function code and must use the HTTP plugin by hand. Routing `call`-kind operations through the same client would let a guest call published `#[server]` functions too. Whether that's done with a custom `server_fn` client over `leptoskit_plugin_http`, or by sending calls over the socket, is a spike in L2.

### 8.5 Versioning and code push

Operation names and argument types are a wire contract with every installed app and every published SDK, and code push makes "installed" a moving target.

- **`lk rpc check`:** compares the current spec with the one recorded in each code push channel's latest release, and with the last published SDK version. Like `lk db check`, it fails CI on a breaking change, such as a removed operation, a new required argument or a changed output type. It suggests aliases (`#[api(since = …)]`) or optional arguments instead. This one check covers live and non-live operations.
- **Releases record the spec:** `lk codepush release` stores the hash of the spec the guest was built against. The server reports `outdated_client` in `welcome` for clients whose spec it can no longer serve, and the client can prompt for an update.

## 9. Scaling

### 9.1 Where the cost is

| Load                 | Driven by                                       | Scales with                                                                                                                                                                                                                          |
| -------------------- | ----------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Idle connections     | Users online                                    | Memory: about 10–30 KiB per connection with its query set. One instance holds tens of thousands                                                                                                                                      |
| Re-runs              | Writes × the distinct dirty queries they affect | Database read capacity. Sharing (§5.4) makes popular queries cost one run per change, not one per viewer                                                                                                                             |
| Fan-out reads        | Commits × instances                             | Each instance reads each committed batch once. That's fine up to tens of instances; the relay (L7) makes it once in total                                                                                                            |
| Jobs                 | Enqueue rate × job duration                     | Workers, and the outside services they call. Queue concurrency caps protect those services. `SKIP LOCKED` claiming handles thousands of jobs a second on Postgres; past that, a dedicated queue backend can implement the same trait |
| Commit sequencing    | Writes to live tables                           | The serialized commit tail. Thousands per second before logical replication is needed                                                                                                                                                |
| Transition bandwidth | Changes × watchers                              | Diffs, and coalescing for slow clients                                                                                                                                                                                               |

### 9.2 Three deployment shapes

**A. One box (SQLite, `fanout = "local"`).**

```text
[ CDN: dist/, wasm, code push ] → [ VPS or Fly machine: app binary (SSR + auth + live) ] → [ SQLite file in WAL mode, Litestream → S3 ]
```

- **Capacity:** thousands of concurrent users, and hundreds of writes per second.
- **Deploying:** one process restart drains its sockets with `reconnect { after_ms: jitter }`, and clients resume within seconds.
- **Cost:** the cheapest option, and the right default for side projects, internal tools and the example app.
- **Limit:** no horizontal scaling. Moving up means switching to Postgres, which portable mode (D2) makes a configuration change.

**B. Stateless instances on Postgres (`fanout = "postgres"`, the default for `--db postgres`).**

```text
     ┌──────────── load balancer (WebSocket-aware, idle timeout > 60 s, no stickiness needed) ────────────┐
     ▼                         ▼                          ▼
instance 1                instance 2                 instance N      (same binary: SSR, RPC, auth, live)
     │  pooled connections for queries and mutations    │
     │  + one direct, unpooled connection each for LISTEN
     ▼                                                   ▼
                  managed Postgres (Neon, RDS, Supabase, Crunchy, Fly Postgres, …)
```

- **No sticky sessions.** Any instance can resume any client from its version. The load balancer only needs WebSocket support and an idle timeout above the 30-second heartbeat.
- **PgBouncer in transaction mode breaks `LISTEN`.** Each instance needs one direct or session-mode connection, `[rpc.live] listen_url_env = "DATABASE_DIRECT_URL"`. `lk db check --live` verifies it can `LISTEN`.
- **Autoscaling:** on connections per instance and `live_rerun_queue_depth`, not only CPU.
- **Rolling deploys** drain sockets in jittered waves (`live.drain()`), so a deploy doesn't stampede the database with re-runs.
- **This shape covers** the large majority of apps, to tens of thousands of concurrent users and around a thousand writes per second to live tables.

**C. Split roles and replication capture (phase L7).**

```text
gateways (--role live)  ←── relay (NATS JetStream or Redis Streams) ←── capture (--role capture: logical replication, pgoutput)
web/RPC (--role web)    ──▶ Postgres primary ──▶ read replicas (re-runs wait for replay ≥ version)
workers (--role worker) ──▶ jobs, crons, pruning
```

- **One binary, three roles.** It's the same binary, started with `--role`, so a small team's deployment can grow into this without rewriting anything.
- **Logical replication capture** replaces the triggers, the commit lock and `NOTIFY`:
  - the commit LSN is the version, so it's ordered by Postgres itself, with no extra writes;
  - one capture process decodes the WAL and publishes batches to the relay;
  - gateways consume from the relay rather than each reading the database.
  - It needs `wal_level = logical` and a replication slot, which most managed Postgres services offer.
- **Re-runs on read replicas:**
  - a replica serves a re-run for version `v` only once its replay position has passed `v`'s commit LSN;
  - otherwise the re-run goes to the primary.
  - The primary then sees writes only, plus re-runs that can't wait.
- **Sharding gateways by query hash.** The relay partitions by table and key range, so each gateway receives only the changes its subscriptions can match.

### 9.3 Not a target in v1

- **Serverless functions** (Lambda, Cloudflare Workers) can't hold a `LISTEN` connection or long-lived sockets. Workers with Durable Objects could host a gateway, but that's a different runtime from axum, so it's a later investigation, not part of this plan.
- **Multi-region active-active writes:** Postgres has one primary. Multi-region reads with region-local gateways and replicas fit shape C.

## 10. Deployment, and what `leptoskit` should ship for it

Today the docs cover hosting static files, server-side rendering and code push, but there's no deploy story for a stateful server. Real time makes one necessary. The proposal follows `lk generate auth`'s approach: generate files the app owns.

**`lk generate deploy --target <fly | docker | compose | kubernetes | systemd>`** writes:

- **A container recipe:** a multi-stage `Dockerfile` that builds the server in release mode, copies `dist/`, and sets the `LEPTOS_*` environment (security-and-scale guide), with a non-root user and a read-only filesystem.
- **A platform file:**
  - `fly.toml`, with a volume for shape A or a Postgres attachment for B;
  - or `compose.yaml` with Postgres;
  - or Kubernetes manifests, with a `Deployment`, a `PodDisruptionBudget` and a `preStop` drain hook;
  - or a systemd unit with socket activation.
- **Health and readiness:**
  - **`/healthz`** answers while the process is up.
  - **`/readyz`** answers only once migrations have run, the `LISTEN` connection is up and the hub has caught up. The load balancer then never routes sockets to an instance that can't serve them.
- **Graceful shutdown:** on `SIGTERM`, readiness fails first, then sockets drain in jittered waves within the platform's grace period, then in-flight mutations finish.
- **Pre-deploy checks:** `lk db check --live` (triggers, `LISTEN`, extensions) and `lk rpc check` (code push and SDK compatibility) run before migrations.
- **Observability:** the server exports OpenTelemetry metrics and traces:
  - `live_connections`, `live_subscriptions`, `live_rerun_seconds`, `live_rerun_queue_depth`, `live_fanout_lag_seconds` (now minus `committed_at`), `live_transition_bytes` and `live_mutation_retries`;
  - each mutation and re-run is a span, labelled with the function's name and the version.

**Configuration in production** stays the same as for auth: secrets only through `_env` fields (`DATABASE_URL`, `DATABASE_DIRECT_URL`, `AUTH_ENCRYPTION_KEYS`). Monosecret or the platform's secret store fills them.

## 11. Testing

- **`rpc::testing`** runs a real `Live` on SQLite in memory or on a Postgres template database (the existing `db::testing`), and gives you clients over the in-memory transport:

  ```rust
  let app = rpc::testing::App::start(my_app::live_router).await;
  let ada = app.client().sign_in_as(&ada).await;
  let grace = app.client().sign_in_as(&grace).await;

  let feed = grace.subscribe(Messages { channel, limit: 50 }).await;
  ada.mutate(SendMessage { channel, body: "hi".parse()? }).await?;
  assert_eq!(feed.next_update().await?.last().unwrap().body.as_str(), "hi");
  ```

- **Matcher soundness:** property tests over random `Expr` trees and rows, on both databases. `eval` must never return `false` when SQL returns `true`.
- **Ordering:** a Postgres stress test runs many concurrent writers, some of which roll back at commit, and checks that a reader polling `version > n` sees every committed transaction exactly once and in order.
- **Policies:** subscriber A must never receive a row that A's direct query wouldn't return. The test covers membership revoked mid-subscription and identity changes over the socket.
- **Protocol:** snapshot tests of the frames, and fuzzing of the server's frame parser.
- **Jobs:** `rpc::testing` runs workers on a controllable clock: `app.run_jobs_until_idle()`, `app.advance(Duration)`, and fault injection that kills a worker mid-step, to check that steps resume and outside calls receive the same key.
- **Platform coverage:** the example (§12) gets a widget journey in `platform_probe` style. Two clients on two platforms see each other's writes in CI, the browser through `wasm-bindgen-test` and the macOS host through its existing fixture, against one test server.
- **`lk dev`** records each live query's read sets and re-run timings, and shows them in a `/_leptoskit/live` page: connections, subscriptions, versions, the slowest re-runs and missing indexes.

## 12. The example: `examples/huddle`

A team chat with a shared task board, built to show every primitive on every platform at once.

**What it has:**

| Feature                                                                                                                 | Primitive it demonstrates                                                                        |
| ----------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------ |
| Channel list in the sidebar; private channels appear when you're added and vanish when you're removed                   | A live query whose result changes because a **policy's subquery** changed, not the queried table |
| Messages with an unread count per channel                                                                               | Two queries kept consistent in one transition (§5.5): the count never disagrees with the list    |
| Sending a message shows instantly, greyed until confirmed                                                               | An optimistic `on_mutate`, then read-your-writes                                                 |
| Kill the network on a phone, send three messages, then reconnect                                                        | The offline queue, receipts (no duplicates), and resume from a version                           |
| "Ada is typing…", and avatars of who's in the channel                                                                   | `#[rpc::channel]` messages and presence                                                          |
| A task board with drag-to-reorder; two people drag the same card                                                        | A serializable mutation with automatic retry; both clients converge                              |
| "Invite by email", with the invite showing _Sending…_ then _Email sent_                                                 | A job enqueued in the mutation's transaction, updating a live row (§3.7)                         |
| A "remind me in 10 minutes" message                                                                                     | `cx.enqueue(…).after(…)`                                                                         |
| "Tip a teammate" in SOL on devnet (behind a `solana` feature): pending at once, then confirmed or failed with **Retry** | Optimistic update, a checkpointed job, `on_failure`, and an inbox notice (§3.7, §3.8)            |
| A bell with unread notices on every device                                                                              | `use_inbox()` (§3.8)                                                                             |
| A "summarize this channel" button                                                                                       | An action calling an outside service, then a mutation                                            |
| Infinite scroll back through history                                                                                    | `use_live_paginated` (L6)                                                                        |
| Admin signs a user out everywhere; their open apps drop to the sign-in screen                                           | Session revocation on live sockets (§7)                                                          |

**One codebase, every target.** Components come from `leptoskit_native` (`View`, `Text`, `Button`, `TextInput`, `VirtualList`), styled with the built-in Tailwind, and sign-in comes from `lk generate auth`. The same guest runs:

- on the web, as a leptos client with hydration;
- on macOS, iOS, Android, Windows and Linux, through the native hosts, with the WebSocket and HTTP plugins granted for the server's origin.

**Seeing it across machines:**

1. **Locally:** `lk dev --all` starts the server on SQLite and opens the web app, the macOS app and the iOS simulator. Sign in as two dev users with `dev_sign_in()`, and type in one to watch the others.
2. **Across real machines:** `lk generate deploy --target fly` followed by `fly deploy` gives shape A on a public URL. Point the native builds at it with `rpc::configure(RpcConfig { origin: "https://huddle-demo.fly.dev".into(), .. })`. Open it on a laptop, a phone and a second laptop.
3. **The scale-out demo:** the same app, with `fly scale count 3` and Postgres, plus a small load generator (`examples/huddle/load`) that opens 5,000 simulated clients across 50 channels and reports p50/p99 time from commit to update. Those numbers become the benchmark the docs quote, and the arena gets a "real-time fan-out" entry.

**Layout:**

```text
examples/huddle/
├── leptoskit.toml            # [db] dialect = "sqlite" (portable mode, so Postgres works too), [auth], [rpc]
├── huddle_db/src/            # tables: channels, memberships, messages, tasks; policies; the live and auth packs
├── huddle_api/src/           # #[rpc::query], #[rpc::mutation], #[rpc::channel], #[rpc::job]: shared by server and clients
├── huddle_app/src/           # components and routes: one guest for every platform
├── huddle_server/src/main.rs # axum: SSR, auth, live, health checks
├── load/                     # the load generator
└── deploy/                   # generated by `lk generate deploy`, committed as an example
```

## 13. Delivery

Each phase is one PR, or a `gh stack` of them. L0 and L1 can start now. They need database phase 4 (policies and packs, done) and the auth sixth layer (the unified client, in review).

| Phase | Contents                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    |
| ----- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| L0    | **`leptoskit_db` prerequisites:** the `Read`/`Write` connection split; `Expr::eval` with three-valued logic, with the soundness property tests on both databases; read-set recording on a `Tracked<C: Read>` connection; read-set entries for `cx.count`/`cx.exists` subqueries (#305), so revoking a membership re-runs what it gated; `HasId` on public types                                                                                                                                                                                             |
| L1    | **Capture:** the `live` table option and `LiveTable`; `#[db(live = skip)]`; the `live` pack's tables; the `db.changes()` feed; trigger, function, sequence and constraint-trigger objects in `leptoskit_db_schema` with renderers, diff and snapshot tests; Postgres commit sealing and `NOTIFY`; SQLite commit versioning; retention pruning; the ordering stress test; `lk db check` additions                                                                                                                                                            |
| L2    | **Server, in `leptoskit_rpc`:** the `server` and `db` features; `#[rpc::query]` and `#[rpc::mutation]` in `leptoskit_rpc_macros`; the registry and the `kind` field in `ApiSpec`; `QueryCx`, `MutationCx`, serializable retries, and the `rpc` pack with receipts; the hub (index, matcher, sharing, snapshot-consistent re-runs, diffs); the WebSocket route and protocol v1; the `Identify` trait and its auth implementation, the `Origin` check, revocation; limits; metrics; `rpc::testing`; the spike on routing `#[server]` calls from native guests |
| L3    | **Client:** transports (browser, guest, in-memory); `leptoskit_query` externally driven entries; `use_live`, `use_live_mutation` with read-your-writes and optimistic updates, `use_live_status`; reconnect and resume; SSR hydration; `lk generate live`; **`examples/huddle` (chat core)**; docs guide `24-real-time.md`; changesets                                                                                                                                                                                                                      |
| L4    | **Channels, presence and streams:** `#[rpc::channel]`, `#[rpc::stream]` with credits; cross-instance delivery over `NOTIFY`; huddle's typing indicators and presence                                                                                                                                                                                                                                                                                                                                                                                        |
| L5    | **Background work:** `#[rpc::job]`, `cx.enqueue` with `.after`, `.at`, `.key` and `.priority`; steps, leases, retries, `on_failure`; queues with concurrency and `serialize_by`; `#[rpc::cron]`; `use_job`; the inbox with `cx.notify` and `use_inbox`; `#[rpc::action]`; `lk jobs` and the jobs page; huddle's invites, reminders, tips and bell                                                                                                                                                                                                           |
| L6    | **Reach:** `use_live_paginated`; the persistent offline queue; the SSE transport and automatic fallback; live methods in every generated SDK (TypeScript, Dart, Python, Go, Rust, CLI) with the per-language protocol runtime; `/asyncapi.json`; `lk rpc check` with code push spec hashes                                                                                                                                                                                                                                                                  |
| L7    | **Scale:** logical replication capture; the relay (NATS JetStream first); `--role`; replica-aware re-runs; partitioned `changes`; the Kubernetes and systemd deploy targets and the live socket drain; the huddle load generator and published numbers                                                                                                                                                                                                                                                                                                      |

**D0, before L0 (decided, R11): deployment.** `lk generate deploy` with its first targets (Docker, Compose and Fly), `/healthz` and `/readyz`, graceful shutdown, and the `LEPTOS_*` container environment, as its own small stack. It doesn't depend on real time, and any app with sign-in and a database needs it now. L7 adds the remaining targets and the live drain.

## Decisions

| #   | Decision                                          | Recommendation                                                                                                             | Trade-off                                                                                                                                                                                                                                      |
| --- | ------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| R1  | Re-run queries, or maintain them incrementally    | **Re-run** (invalidate, don't replicate), with sharing and keyed diffs                                                     | Incremental view maintenance (as Materialize does) is cheaper per change but can't express arbitrary Rust query functions or policies. Re-running is simple, correct, and is what Convex does                                                  |
| R2  | How writes are captured                           | **Decided: triggers generated by migrations**; logical replication as the L7 scale path                                    | Triggers catch every writer and work on both databases, at the cost of extra write I/O. ORM-side capture misses raw SQL and other services. Replication needs `wal_level = logical` and slot management                                        |
| R3  | Ordering                                          | **Decided: a global version allocated at commit under a transaction-scoped advisory lock**; the ceiling is accepted for v1 | Ordered and simple, but the commit tail is serialized: a ceiling of thousands of commits per second, lifted by L7                                                                                                                              |
| R4  | Transport                                         | **WebSocket primary; SSE read-only fallback on the web**                                                                   | One extra route and transport, for proxies that strip upgrades and for simple consumers. Native stays WebSocket-only                                                                                                                           |
| R5  | Functions: leptos server functions, or a registry | **A registry of named operations, as kinds of `#[api]` operation** with generated stubs                                    | Native guests can't contain server functions (auth-plan, sixth layer), so a named registry is the only API that is the same everywhere. Server functions can still call into it                                                                |
| R6  | Which tables are live                             | **Decided: opt-in `live`, enforced by the `LiveTable` bound**                                                              | One attribute per table, against paying for capture on tables nobody watches. Forgetting it is a compile error, not a silent stale UI                                                                                                          |
| R7  | Cross-instance fan-out                            | **Postgres `LISTEN/NOTIFY`** by default; relay in L7                                                                       | No Redis to run, unlike Serverpod. Needs one unpooled connection per instance, and every instance reads each batch                                                                                                                             |
| R8  | Mutation isolation                                | **`SERIALIZABLE` with automatic retry**, plus receipts for exactly-once                                                    | Bodies must be retry-safe, so side effects move to `after_commit`, `schedule` and actions, as in Convex                                                                                                                                        |
| R9  | Wire format                                       | **JSON v1**; postcard negotiated later                                                                                     | Readable in devtools, and matches the generated clients. Diffs keep sizes small                                                                                                                                                                |
| R10 | Ephemeral traffic                                 | **Typed channels with presence**, never stored                                                                             | At most once; anything that must survive belongs in a table and a live query                                                                                                                                                                   |
| R11 | When deployment tooling ships                     | **Decided: first, as its own stack (D0)**, before real time                                                                | A small stack that real time doesn't block. It gives auth and the huddle example somewhere to run across machines from the start                                                                                                               |
| R12 | Where the code lives                              | **Decided: in `leptoskit_rpc`**, behind features; change capture in `leptoskit_db`                                         | One crate for every way of reaching the app's data, and one spec. The cost is a bigger crate, which features keep out of guest builds. Capture stays in the database crate because it's schema work, and is useful without the RPC             |
| R13 | Job delivery guarantee                            | **Decided: at least once, with checkpointed steps and per-step idempotency keys**; exactly once only inside the database   | Nothing can make an email or a payment exactly once across a crash by itself. Steps plus keys get as close as the outside service allows, and the docs say which services accept keys                                                          |
| R14 | Where jobs are stored                             | **Decided: in the app's own database** (`rpc.jobs`), enqueued in the caller's transaction                                  | No queue service to run, and no lost or phantom jobs, because a job exists exactly when its mutation committed. Postgres `SKIP LOCKED` handles thousands of jobs a second; heavier loads can move to a dedicated backend behind the same trait |
| R15 | Reaching closed apps                              | **Decided: out of scope here; remote push is its own plan**, delivered through the inbox                                   | APNs, FCM and Web Push need credentials, device registration and a new plugin. The inbox works without them and gives push a single place to plug in                                                                                           |

## Risks

- **Re-run storms.** A popular query on an unindexed filter, or a write to a hot row watched by many different queries. Mitigations: sharing (§5.4), coalescing, the missing-index warnings, queue-depth metrics, and docs that steer large aggregates to counters maintained in mutations, as Convex's do.
- **Commit-lock throughput.** If an app outgrows it before L7 ships, it has no workaround. Measure it in L1 and publish the number.
- **Trigger drift.** Someone edits the database by hand. `lk db check --live` detects it, and migrations always re-render triggers from the schema.
- **`NOTIFY` limits.** Notifications are lost if a listener's connection drops, and the queue is finite. Mitigation: listeners treat `NOTIFY` as a hint and always read `commits` from their last version. A dropped `LISTEN` connection triggers a catch-up read, and a stalled listener is visible in `live_fanout_lag_seconds`.
- **Determinism is a convention.** A live query that reads the clock or calls out will show stale results. Debug builds flag `SystemTime` and HTTP plugin use inside query bodies where they can, and the docs lead with it.
- **Policy coverage of the matcher.** Subqueries make many policies match `Unknown`, which means more re-runs, but never a leak. Revisit with subquery-aware matching (holding membership sets in the hub) if huddle's numbers say so.
- **Guest limits.** The WebSocket plugin's 1 MiB frames and pending-byte budget bound result sizes. The server-side limit is lower by default, and diffs keep frames small.
- **Jobs that touch the outside world twice.** At-least-once delivery is safe only when outside calls carry `cx.key()`. Debug builds warn when a job calls the HTTP client outside a `cx.step`, and the guide lists which common services accept idempotency keys.
- **Stuck rows.** An app row left `pending` because its job failed silently. `on_failure` exists for this, and `lk db check` warns when a job that writes a status column has none.
- **Scope.** L0 to L3 alone are a usable Convex-style core. L4 to L7 are independent, and each can wait.
