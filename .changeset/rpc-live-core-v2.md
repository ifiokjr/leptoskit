---
"release": minor
---

# One live hub, server-sent events, and exact versions

A server now reads the database's change log once for every connection, and the live connection works where a WebSocket can't.

- **One hub per server.** `rpc::server::Live` reads each commit once, finds the subscriptions it could change through an index by table, and re-runs only those. Subscriptions to the same query with the same arguments by the same caller share one re-run and one diff, on any number of connections. Each connection receives one transition per batch of commits. `LiveStats` gains `commits` (read once each) and `resumes`.
- **Server-sent events.** `GET /live/events` streams the same frames as server-sent events, with each transition's version as its event id, and `POST /live/frames?connection=<id>` takes the client's frames. Both are authenticated like the socket, and a post must come from the stream's caller. `Last-Event-ID` resumes. The browser client switches to them after two failed WebSocket upgrades; native apps stay on the WebSocket.
- **Read-your-writes from the commit itself.** A mutation's `done.version` is the version its commit took in the change log, from the new `Tx::commit_version()`. Every transition names the version it reflects, and a connection waiting for a version that changed none of its results gets an empty transition, so mutations never hang. `LiveClient::mutate`, `LiveClient::act`, `use_live_mutation` and `use_live_action` resolve once the client has applied that version.
- **Actions over the connection.** The new `act` frame runs an `#[rpc::action]` with the same identity and errors as `POST /api/<name>`.
- **Resume.** `hello { resume: { version, queries } }` subscribes again without a full reload: the server sends only the results that changed since `version`, or everything when the log was pruned past it. The client keeps its last applied version and resumes after a reconnect or a drain.
- **`leptoskit_db`.** `Tx::commit_version()`, `ChangeLog::next_ready()` and `Context::is_empty()`.
