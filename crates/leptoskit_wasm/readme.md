# leptoskit_wasm

> The on-device wasm runtime host: the code push runtime for native apps.

The app shipped through the store is a thin platform shell; the _interface_ is a wasm module fetched over the air. This crate runs that module in [`wasmi`](https://crates.io/crates/wasmi) — a pure interpreter, which is legal on iOS where JIT compilation is not — and drives a `leptoskit_native::NativeSurface` through a small, versioned ABI.

## The ABI

The module imports from the `leptoskit` module:

| Function                            | Meaning                                 |
| ----------------------------------- | --------------------------------------- |
| `node_create(id, kind)`             | create a widget                         |
| `node_set_props(id, ptr, len)`      | set props from JSON in wasm memory      |
| `node_insert(parent, child, index)` | attach a child                          |
| `node_remove(parent, child)`        | detach a child                          |
| `node_delete(id)`                   | destroy a widget                        |
| `commit()`                          | hand the assembled tree to the renderer |

The module exports `memory`, `leptoskit_main()` (build and commit the initial tree) and `leptoskit_event(id, kind, ptr, len)` (rebuild after an event, with an optional JSON payload for text events). Every entry point is fuel-metered, so a bad bundle cannot hang the device, and every commit is diffed by the renderer, so a rebuild is cheap.

Because updates are just new wasm bundles on a channel, shipping a fix works exactly like React Native's code push: no store review, instant rollback, and the same `leptoskit_codepush` manifests and deltas as every other artifact.
