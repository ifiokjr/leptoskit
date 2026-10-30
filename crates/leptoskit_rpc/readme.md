<p align="center">
  <a href="https://ifiokjr.github.io/leptoskit/"><img src="https://raw.githubusercontent.com/ifiokjr/leptoskit/main/docs/src/assets/mark.svg" alt="leptoskit" width="64"></a>
</p>

# leptoskit_rpc

> An ergonomic API over leptos server functions.

Leptos server functions are already typed, isomorphic RPC. This crate adds the layer around the call:

- `query` — a retrying, reactive read with an error type that keeps loading and failure in one value;
- `mutation` / `mutation_with_retry` — leptos actions with unified errors; mutations only retry when you say so, because blindly re-sending a POST is usually wrong;
- `FnError` — failures classified as network, invalid, server, or decode, so "offline" and "your input was rejected" stop looking identical in the UI; only network failures are retried, because a server error is also how a function says "not found";
- `RetryPolicy` — exponential backoff with a cap, tuned defaults for reads and flaky mobile networks, waited out on `leptoskit_query`'s timer in the browser, on a Tokio runtime, or without one.

## Queries and mutations on every platform

`#[rpc::query]` and `#[rpc::mutation]` declare operations that a native app's Wasm guest, a browser page, and the server's own rendering all call the same way: `rpc::client::call(Notes { limit: 20 })`. Each function becomes a struct of its arguments on every target, and only the server build (`server`, `db`, and `postgres` or `sqlite`) keeps the body. A query reads the database as the caller, read-only by type; a mutation runs in one serializable transaction, retried on conflicts, and runs once per `Idempotency-Key`. `rpc::server::Server` serves them at `POST /api/<name>`, `rpc::Identify` says who is calling (`leptoskit_auth` implements it), and `use_rpc_query` and `mutation_scope` put them in `leptoskit_query`'s cache.

## Live queries

With the `live` feature (on by default with `postgres` or `sqlite`), a query can stay open. A client subscribes by operation name and arguments over `GET /live`; the server runs the query as that client, records the read set its own `Tracked` connection produced, and sends the result at the newest committed version. Every write to a live table is captured by `leptoskit_db`; the hub asks each subscription's read set whether the change could have affected it, re-runs only those it could, and sends the new value or a keyed patch. Re-runs use the subscriber's policies, so a client never receives a row its own query couldn't see.

```rust,ignore
let rpc = rpc::Server::new(db.clone()).identify(auth.clone());
rpc.start();                       // the change poll loop
let app = Router::new().merge(rpc.router());   // POST /api/<name>, GET /live
```

`LiveHub` is usable without a socket, which is how the tests and an in-process client drive it. The wire protocol is `leptoskit.live.v1.json`: `hello`, `sub`, `unsub`, `mutate` and `ping` from the client; `welcome`, `transition`, `done`, `auth_status`, `reconnect` and `pong` from the server.

## Publishing the RPC to any language

`#[api]` above `#[server]` publishes a function: a stable `/api/<name>` route, a JSON Schema description of its arguments and result (through `schemars`), and its doc comment — including the `# Arguments` section — as documentation for every argument. `#[api(internal)]` keeps a function callable but unpublishes it, and websockets, streams, and other shapes no client could call are compile errors. With `ssr` enabled next to `leptos/ssr` (`rpc-ssr` through the `leptoskit` crate) and `leptoskit_rpc::spec::export_if_requested()` first in `main`, which also refuses to start a server where two functions share a route, `leptoskit rpc generate` writes typed clients that speak the RPC's own protocol for TypeScript, Python, Go, Rust, Dart, and the command line, plus the OpenAPI 3.1 document. See the book's "Sharing the RPC" chapter for the walkthrough.
