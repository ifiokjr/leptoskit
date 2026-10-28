---
"release": minor
---

# One dependency for the server

A server package now depends on `leptoskit` alone. The new `server` feature re-exports the server toolkit under `leptoskit::server` together with the crates every deployment links — `axum`, `tokio`, `tower`, `tower-http`, and `leptos_axum` — at the versions leptoskit was built against; `tokio` carries the `macros` and `rt-multi-thread` features, so `use leptoskit::server::tokio;` is all `#[tokio::main]` needs, and `tower` and `tower-http` carry the limit, load-shed, and timeout layers. `leptoskit_axum` re-exports the same four crates and gains `serve_router` and `serve_listener`, which serve an app's own router until `SIGINT` or `SIGTERM` and finish the requests in flight first, the way `serve` always has.

`leptos`, `leptos_router`, and `leptos_meta` are re-exported behind the `leptos` feature, and the new `csr`, `hydrate`, `ssr`, and `islands` features forward to them, so a package can write `hydrate = ["leptoskit/hydrate"]` instead of naming leptos; `server` implies `ssr`. `leptoskit build` turns islands on through `leptoskit/islands` when a package reaches leptos that way, and only requires a direct dependency on one of the two. A crate that uses leptos's macros still depends on `leptos` itself, because they expand to `::leptos::` paths.

`leptoskit_codepush` re-exports `ed25519_dalek` and `semver`, whose types sign and number every release.

`leptoskit new` generates a server and a backend that depend on `leptoskit` only, and the server now serves `pkg/` and `assets/` with their precompressed copies and stops cleanly on `SIGTERM`.
