---
"release": minor
---

# Live queries over a WebSocket

`leptoskit_rpc` gains the live side of its operations, on top of the database's live invalidation feed.

- **`rpc::server::Live`.** `GET /live` speaks `leptoskit.live.v1.json`: subscribe to registered queries by name, run mutations on the same socket, and receive what changed. It identifies callers with the same `rpc::Identify` implementation the HTTP routes use, checks the upgrade's `Origin`, re-runs queries as the caller, and sends only results that changed. `Server::live()` builds one from an existing server, and `Live::drain()` reconnects every socket with jitter for a deployment.
- **Keyed patches.** A list of rows with an `id` travels as removals and positioned upserts; everything else travels whole. A slow connection gets the latest state whole instead of every intermediate step.
- **Limits.** 256 subscriptions per connection, 512 KiB per result, and 1 MiB per frame, enforced before a query can grow the server without bound.
- **`rpc::use_live`.** The screen-facing hook is `use_rpc_query` plus a subscription: the first value comes from the ordinary client (HTTP, or in process while rendering), and updates are written into the same `leptoskit_query` cache, so suspense, SSR, hydration and devtools work unchanged. The client reconnects with backoff, resubscribes, and re-sends pending mutations with their idempotency keys.
- **Transports.** The browser uses `leptoskit_plugin_websocket`'s browser client; a native guest uses its guest client. Without either, `use_live` is exactly `use_rpc_query`.
- **Facade.** `leptoskit` gains `rpc-live`.
- **Docs.** "Real-time screens" covers opting tables in, serving the socket, watching a query, what arrives, reconnecting, and the limits.
