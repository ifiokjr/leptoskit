# Web

The browser is leptoskit's home platform: the interface compiles to wasm, the server renders HTML with SSR, and hydration attaches the client to the server's output.

## What a web app looks like

A leptoskit web project is a leptos workspace: a `cdylib` crate for the client (wasm), a binary crate for the server, and a `leptoskit.toml`:

```toml
[app]
name = "my_app"
default_platform = "web"

[serve]
address = "127.0.0.1"
port = 3000
```

The client crate marks itself with `crate-type = ["cdylib"]`, which is how `leptoskit` finds it — no configuration needed unless several crates produce cdylibs, in which case pin it:

```toml
[build]
client_package = "my_app_client"
```

## Building

```bash
leptoskit build --release --platform web --out dist
```

The pipeline is cargo-leptos' job with Dioxus-CLI speed (see the [CLI readme](../../crates/leptoskit_cli/readme.md) for the full speed design): the server and the wasm client compile in parallel, wasm-bindgen runs in-process keyed on the wasm content hash so unchanged compiles skip it, `wasm-opt` runs only in release and only when its input moved, and every phase's fingerprint persists in `target/leptoskit/cache.json`. A no-change rebuild measures ~155ms; a one-line source change ~300ms.

The output lands in `dist/`: the server binary, `dist/pkg/` with the wasm-bindgen glue, and `public/` assets copied content-hashed.

## Developing

```bash
leptoskit dev
```

One process runs the watcher, the cached build pipeline, and the dev server together. Save a file; the watcher debounces the editor's burst of writes, rebuilds only the phases whose inputs moved, and the server keeps serving throughout.

## Serving

```bash
leptoskit serve
```

Static assets with pre-compressed gzip, an SPA fallback to `index.html`, a health check at `/_leptoskit/health`, and — when a release store is configured — the code push registry under `/api/codepush`. SSR and leptos server functions mount through `leptos_axum` in your server binary; Axum is the one supported server.

## SSR and server functions

Server-side rendering runs in your server binary with the standard leptos toolchain, and `leptoskit_fns` wraps the client side of server functions with retry and error classification so "offline" and "invalid input" stop looking identical in the UI:

```rust
let todos = query(RetryPolicy::reads(), || async {
    list_todos().await.map_err(FnError::from)
});
```

The same call works from the browser, from SSR, and from a native shell — the transport is HTTP everywhere.
