# Web

The browser is leptoskit's home platform: the interface compiles to wasm, the server renders HTML with SSR, and hydration attaches the client to the server's output, either the whole page or only its islands.

## What a web app looks like

A leptoskit web project is a leptos workspace: a `cdylib` crate for the client (wasm), a binary crate for the server, and a `leptoskit.toml`:

```toml
[app]
name = "my_app"
default_platform = "web"
render = "ssr"

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

The pipeline does cargo-leptos' job, kept fast (the [build guide](../guide/04-building.md) has the details). cargo compiles the server and the wasm client and decides for itself what to recompile. wasm-bindgen then runs in process, keyed on the compiled wasm's content hash, so an unchanged compile skips it. `wasm-opt` runs only on release builds, and only when its input changed.

The output lands in `dist/`: `dist/pkg/` holds the wasm-bindgen glue and the bound wasm, next to your `public/` assets. The server binary stays in cargo's target directory.

## Developing

```bash
leptoskit dev
```

One process runs the watcher, the cached build pipeline, and the dev server together. Save a file; the watcher debounces the editor's burst of writes, rebuilds, and the server keeps serving throughout.

## Serving

```bash
leptoskit serve
```

Static assets with pre-compressed gzip, an SPA fallback to `index.html`, a health check at `/_leptoskit/health`, and — when a release store is configured — the code push registry under `/api/codepush`. SSR and leptos server functions mount through `leptos_axum` in your server binary; Axum is the one supported server.

## SSR and server functions

`app.render` picks how pages render: `"csr"` in the browser, `"ssr"` on the server with full hydration, or `"islands"` on the server with only the interactive islands hydrated. In the server-rendering modes, leptoskit builds the server with its `ssr` feature and the client with its `hydrate` feature, then `leptoskit dev` and `leptoskit serve` run your server binary with the `LEPTOS_*` environment set. The [small bundles guide](../guide/07-small-bundles.md) shows a complete islands app.

Server-side rendering runs in your server binary with the standard leptos toolchain, and `leptoskit_fns` wraps the client side of server functions with retry and error classification so "offline" and "invalid input" stop looking identical in the UI:

```rust,ignore
let todos = query(RetryPolicy::reads(), || async {
    list_todos().await.map_err(FnError::from)
});
```

The same call works from the browser, from SSR, and from a native shell — the transport is HTTP everywhere.
