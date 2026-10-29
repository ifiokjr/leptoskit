<p align="center">
  <a href="https://ifiokjr.github.io/leptoskit/"><img src="https://raw.githubusercontent.com/ifiokjr/leptoskit/main/docs/src/assets/mark.svg" alt="leptoskit" width="64"></a>
</p>

# leptoskit_rpc_macros

> The `#[api]` attribute behind `leptoskit_rpc`. Use it as `leptoskit_rpc::api`.

A leptos `#[server]` function is already typed RPC between your app and its server, but nothing outside the app can discover it: its default route embeds a hash of the manifest path, so it changes between checkouts, and nothing describes its arguments. `#[api]` publishes the function so `leptoskit rpc generate` can write typed clients (TypeScript, Python, Go, Rust, Dart, and a command line) and an OpenAPI document.

Placed above `#[server]`, it leaves the RPC itself alone and adds a stable route and a description:

```rust,ignore
use leptoskit_rpc::api;
use leptos::prelude::ServerFnError;
use leptos::server;

/// Lists every todo.
///
/// # Arguments
///
/// * `limit` - How many to return.
#[api(tag = "todos")]
#[server]
pub async fn list_todos(limit: Option<u32>) -> Result<Vec<Todo>, ServerFnError> {
	// ...
}
```

This function is served at `/api/list_todos`, and its doc comment and `# Arguments` section become the documentation of the generated client's method and parameters. Every argument type, and the `Ok` type of the return value, must implement `schemars::JsonSchema` (one derive next to `Serialize` and `Deserialize`). Reach the macro through `leptoskit_rpc::api`, or `leptoskit::rpc::api` with `#[api(crate = "leptoskit::rpc")]`. The book's "Sharing the RPC" chapter is the full walkthrough, and [`examples/harbor`](https://github.com/ifiokjr/leptoskit/tree/main/examples/harbor) is a working app.

The route is pinned to a stable `/api/<name>`, the function name or `#[api(name = "...")]`, unless `#[server]` sets an `endpoint`. What no generated client could call is a compile error that points at its cause: websocket protocols, streaming and multipart encodings, `#[server(flatten)]` arguments, the positional `#[server]` form without an endpoint, and raw-identifier function names. Under the `ssr` feature, enabled through `leptoskit_rpc/ssr`, a description is registered with the route, encodings, doc comment, argument docs (from each argument's doc comment or the `# Arguments` section), and a JSON Schema thunk for every argument and the return type, which `leptoskit_rpc::spec::collect` exports. `#[server(skip)]` arguments are left out.

Options: `internal` (callable, just not publicized), `name = "..."`, `tag = "..."` (repeatable), and `crate = "..."`.
