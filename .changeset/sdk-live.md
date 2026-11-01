---
"release": minor
---

# Watch, mutate and join live from every generated client

Every client `leptoskit rpc generate` writes now speaks the live socket, not only one-shot HTTP. TypeScript gets `api.notes.subscribe(args, onValue)`, Python `for value in api.notes.watch(...)` (and `async for` on `AsyncClient`), Go `client.SubscribeNotes(ctx, ...)`, Rust `api.notes_watch(...)` as a `Stream`, and Dart `api.notes.watch(...)` as a `Stream` for `StreamBuilder`. Mutations gain a live form that resolves once the client's own subscriptions show the write, channels can be joined with presence and signals, and the command line gets `--watch` on queries and a `join` command per channel, printing one line of JSON per value.

Each client carries a small protocol runtime next to its code: one WebSocket, keyed patches applied atomically, bearer tokens renewed on expiry, heartbeats, and reconnects with jittered backoff that subscribe again and re-send a lost mutation with the same key so it runs once.

`#[api]` above `#[rpc::channel]` publishes the channel. The spec, now version 3, records channels with their join, message and presence schemas and the live path, and the AsyncAPI document describes them.
