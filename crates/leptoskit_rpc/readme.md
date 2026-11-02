<h1 align="center">leptoskit_rpc</h1>

<p align="center">
  <a href="https://ifiokjr.github.io/leptoskit/"><img src="https://raw.githubusercontent.com/ifiokjr/leptoskit/main/docs/src/assets/mark.svg" alt="leptoskit" width="64"></a>
</p>

<p align="center">
  <strong>leptoskit_rpc</strong> is the app's operations layer: leptos server functions with retries and error classification, plus queries and mutations that run on the database and publish typed clients for other languages.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://docs.rs/leptoskit_rpc"><strong>Docs</strong></a> ·
  <a href="https://github.com/ifiokjr/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<p align="center">
  <a href="https://crates.io/crates/leptoskit_rpc"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_rpc.svg"></a>
  <a href="https://docs.rs/leptoskit_rpc"><img alt="docs.rs" src="https://docs.rs/leptoskit_rpc/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<br>

## Overview

Leptos server functions are already typed, isomorphic RPC. This crate adds the layer around the call:

- `query` — a retrying, reactive read with an error type that keeps loading and failure in one value;
- `mutation` / `mutation_with_retry` — leptos actions with unified errors; mutations only retry when you say so, because blindly re-sending a POST is usually wrong;
- `FnError` — failures classified as network, invalid, server, or decode, so "offline" and "your input was rejected" stop looking identical in the UI; only network failures are retried, because a server error is also how a function says "not found";
- `RetryPolicy` — exponential backoff with a cap, tuned defaults for reads and flaky mobile networks, waited out on `leptoskit_query`'s timer in the browser, on a Tokio runtime, or without one.
- the `devtools` feature — every retry attempt recorded with its error class, for leptoskit's devtools calls timeline.

## Queries and mutations on every platform

`#[rpc::query]` and `#[rpc::mutation]` declare operations that a native app's Wasm guest, a browser page, and the server's own rendering all call the same way: `rpc::client::call(Notes { limit: 20 })`. Each function becomes a struct of its arguments on every target, and only the server build (`server`, `db`, and `postgres` or `sqlite`) keeps the body. A query reads the database as the caller, read-only by type; a mutation runs in one serializable transaction, retried on conflicts, and runs once per `Idempotency-Key`. `rpc::server::Server` serves them at `POST /api/<name>`, `rpc::Identify` says who is calling (`leptoskit_auth` implements it), and `use_rpc_query` and `mutation_scope` put them in `leptoskit_query`'s cache.

## Publishing the RPC to any language

`#[api]` above `#[server]` publishes a function: a stable `/api/<name>` route, a JSON Schema description of its arguments and result (through `schemars`), and its doc comment — including the `# Arguments` section — as documentation for every argument. `#[api(internal)]` keeps a function callable but unpublishes it, and websockets, streams, and other shapes no client could call are compile errors. With `ssr` enabled next to `leptos/ssr` (`rpc-ssr` through the `leptoskit` crate) and `leptoskit_rpc::spec::export_if_requested()` first in `main`, which also refuses to start a server where two functions share a route, `leptoskit rpc generate` writes typed clients that speak the RPC's own protocol for TypeScript, Python, Go, Rust, Dart, and the command line, plus the OpenAPI 3.1 document. See the book's "Sharing the RPC" chapter for the walkthrough.

## How it fits

It is the boundary between shared screens and the server. The same call runs during SSR, in the browser, and in a native guest; `leptoskit_rpc_codegen` turns the specification into TypeScript, Python, Go, Rust, Dart, and CLI clients.

## Use it

```rust,ignore
#[leptoskit_rpc::api]
async fn list_notes() -> Result<Vec<Note>, ServerFnError> {
    leptos::server::list_notes().await
}
```

## Learn more

- [Crate documentation](https://docs.rs/leptoskit_rpc)
- [leptoskit documentation](https://ifiokjr.github.io/leptoskit/)
- [Repository](https://github.com/ifiokjr/leptoskit)
- [Server functions](https://ifiokjr.github.io/leptoskit/guide/06-server-functions.html)
- [Sharing the RPC](https://ifiokjr.github.io/leptoskit/guide/13-sharing-the-rpc.html)
