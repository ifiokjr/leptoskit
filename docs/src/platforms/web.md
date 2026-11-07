# Web

The browser is leptoskit's home platform: the interface compiles to wasm, the server renders HTML with SSR, and hydration attaches the client to the server's output, either the whole page or only its islands.

## What a web app looks like

Application screens can use the [shared component builders](../guide/07-shared-components.md) instead of HTML elements. Enable `leptoskit`'s optional `web` feature and pass the same `Element` tree used by a native surface to `leptoskit::web::render`. Wrap that call in a reactive Leptos closure when the tree reads signals. The adapter creates semantic DOM controls and emits the built-in Tailwind stylesheet, including responsive and dark-mode media queries.

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

## Transforms and animations

Transforms, transitions, keyframes, and entering [animations](../guide/32-animations.md) become CSS, so the browser runs them. Springs become a `linear()` easing sampled from the spring. `exiting` and `layout_transition` don't run in the browser yet: a removed element disappears at once, and a moved one jumps.

## Rive animations

[Rive animations](../guide/20-rive.md) can run on a browser canvas through `leptoskit_rive`. It uses an app-supplied official runtime, suspends hidden and offscreen rendering, and releases resources when the player is dropped. Keep its meaningful content accessible in HTML beside the canvas.

## Building

```bash
leptoskit build --release --platform web --out dist
```

The pipeline does cargo-leptos' job, kept fast (the [build guide](../guide/04-building.md) has the details). cargo compiles the server and the wasm client and decides for itself what to recompile. wasm-bindgen then runs in process, keyed on the compiled wasm's content hash, so an unchanged compile skips it. `wasm-opt` runs only on release builds, and only when its input changed.

The output lands in `dist/`: `dist/pkg/` holds the wasm-bindgen glue and the bound wasm, next to the files from `web/` and, under `dist/assets/`, your shared `assets/`. The server binary stays in cargo's target directory.

## Developing

```bash
leptoskit dev
```

One process runs the watcher, the cached build pipeline, and the dev server together. Save a file; the watcher debounces the editor's burst of writes, rebuilds, and the server keeps serving throughout.

## Serving

```bash
leptoskit serve
```

Static assets with their precompressed brotli and gzip copies, an SPA fallback to `index.html`, and a health check at `/_leptoskit/health`. The code push registry is a separate server: `leptoskit codepush serve`. SSR and leptos server functions mount through `leptos_axum` in your server binary; Axum is the one supported server.

## SSR and server functions

`app.render` picks how pages render: `"csr"` in the browser, `"ssr"` on the server with full hydration, or `"islands"` on the server with only the interactive islands hydrated. In the server-rendering modes, leptoskit builds the server with its `ssr` feature and the client with its `hydrate` feature, then `leptoskit dev` and `leptoskit serve` run your server binary with the `LEPTOS_*` environment set. The [small bundles guide](../guide/09-small-bundles.md) shows a complete islands app.

Server-side rendering runs in your server binary with the standard leptos toolchain, and `leptoskit_rpc` wraps the client side of server functions with retry and error classification so "offline" and "invalid input" stop looking identical in the UI:

```rust,ignore
let todos = query(RetryPolicy::reads(), || async {
    list_todos().await.map_err(FnError::from)
});
```

The same call works in the browser and during SSR. Native apps run their interface as a Wasm guest without server-function code, so this call doesn't exist there; see [server functions](../guide/06-server-functions.md) for how a native app reaches the server.

## Service workers

A Progressive Web App needs a service worker for offline caching and Web Push. Enable leptoskit's `service-worker` feature and ship a worker script with your web assets (for example `web/sw.js`). The page registers it and talks to it in Rust:

```rust,ignore
use leptoskit::service_worker::prelude::*;

let registration = register("/sw.js", RegisterOptions::default()).await?;
let _ = ready().await?;

send(&WorkerMessage::SetCacheRules {
	rules: vec![
		CacheRule::new("static-v1", Strategy::cache_first()).prefixes(["/assets/"]),
		CacheRule::new("pages", Strategy::network_first_within(2_000)),
	],
})?;

let push = registration.push_manager()?;
let subscription = push
	.subscribe(&PushOptions::new(vapid_public_key))
	.await?;
```

`Strategy` values describe cache-first, network-first, stale-while-revalidate, network-only, and cache-only behaviour. Post them to your worker script (or read them inside one) so routes stay defined in one place. `caches` helpers open Cache Storage from the page when you need to precache or inspect entries yourself.

This is the page side only: you still write the worker script that handles `fetch`, `push`, and `message` events. Calls return `Unavailable` outside a secure browser context, so the same app code compiles in tests and on native targets. Browser extensions use a different background worker through [extensions](extension.md); do not mix the two.
