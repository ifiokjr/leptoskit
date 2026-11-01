<h1 align="center">leptoskit_rpc_macros</h1>

<p align="center">
  <a href="https://ifiokjr.github.io/leptoskit/"><img src="https://raw.githubusercontent.com/ifiokjr/leptoskit/main/docs/src/assets/mark.svg" alt="leptoskit" width="64"></a>
</p>

<p align="center">
  <strong>leptoskit_rpc_macros</strong> provides the `#[api]`, `#[query]`, and `#[mutation]` attributes behind `leptoskit_rpc`.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://docs.rs/leptoskit_rpc_macros"><strong>Docs</strong></a> ·
  <a href="https://github.com/ifiokjr/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<p align="center">
  <a href="https://crates.io/crates/leptoskit_rpc_macros"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_rpc_macros.svg"></a>
  <a href="https://docs.rs/leptoskit_rpc_macros"><img alt="docs.rs" src="https://docs.rs/leptoskit_rpc_macros/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<br>

## Overview

A leptos `#[server]` function is already typed RPC between your app and its server, but nothing outside the app can discover it: its default route embeds a hash of the manifest path, so it changes between checkouts, and nothing describes its arguments. `#[api]` publishes the function so `leptoskit rpc generate` can write typed clients (TypeScript, Python, Go, Rust, Dart, and a command line) and an OpenAPI document.

Placed above `#[server]`, it leaves the RPC itself alone and adds a stable route and a description.

This function is served at `/api/list_todos`, and its doc comment and `# Arguments` section become the documentation of the generated client's method and parameters. Every argument type, and the `Ok` type of the return value, must implement `schemars::JsonSchema` (one derive next to `Serialize` and `Deserialize`). Reach the macro through `leptoskit_rpc::api`, or `leptoskit::rpc::api` with `#[api(crate = "leptoskit::rpc")]`. The book's "Sharing the RPC" chapter is the full walkthrough, and [`examples/harbor`](https://github.com/ifiokjr/leptoskit/tree/main/examples/harbor) is a working app.

The route is pinned to a stable `/api/<name>`, the function name or `#[api(name = "...")]`, unless `#[server]` sets an `endpoint`. What no generated client could call is a compile error that points at its cause: websocket protocols, streaming and multipart encodings, `#[server(flatten)]` arguments, the positional `#[server]` form without an endpoint, and raw-identifier function names. Under the `ssr` feature, enabled through `leptoskit_rpc/ssr`, a description is registered with the route, encodings, doc comment, argument docs (from each argument's doc comment or the `# Arguments` section), and a JSON Schema thunk for every argument and the return type, which `leptoskit_rpc::spec::collect` exports. `#[server(skip)]` arguments are left out.

Options: `internal` (callable, just not publicized), `name = "..."`, `tag = "..."` (repeatable), and `crate = "..."`.

## How it fits

Use them through `leptoskit_rpc`; an `#[api]` publishes a leptos server function to generated clients, while queries and mutations add the client-side retry and cache behavior.

## Use it

```rust,ignore
#[leptoskit_rpc::query]
async fn note(id: u64) -> Result<Note, ServerFnError> {
    leptos::server::note(id).await
}
```

## Learn more

- [Crate documentation](https://docs.rs/leptoskit_rpc_macros)
- [leptoskit documentation](https://ifiokjr.github.io/leptoskit/)
- [Repository](https://github.com/ifiokjr/leptoskit)
- [Server functions](https://ifiokjr.github.io/leptoskit/guide/06-server-functions.html)
