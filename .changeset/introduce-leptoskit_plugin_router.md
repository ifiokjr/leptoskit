---
leptoskit_plugin_router: major
---

# leptoskit_plugin_router: file routes as a build plugin

`leptoskit_plugin_router` is the file router as a leptoskit plugin: it generates the typed `Route` module from the files under `src/routes/` before cargo runs, and answers `plugin:router routes`.

Turn it on with a `[[plugins]]` entry named `router` in `leptoskit.toml`; `leptoskit dev`, `build`, and `native` then rewrite `src/routes.rs` only when the routes change, and `leptoskit plugin:router routes --check` fails when the committed module is stale. Apps don't depend on this crate, since the CLI links it; the runtime and the `Route` derive live in `leptoskit_router`.
