---
"release": minor
---

# Split lazy code into chunks that load on demand

`build.split = true` runs wasm-split over the client in process before wasm-bindgen: every `#[lazy]` function, lazy route, and `#[island(lazy)]` moves into a wasm chunk that loads the first time it's needed. leptoskit links the client with `--emit-relocs`, writes the chunks, their loader, and the `__wasm_split_manifest.json` that leptos' `HydrationScripts` reads to preload chunks, binds the main module, and runs `wasm-opt -Oz` over every chunk in release. The bundle report marks chunks as lazy and leaves them out of the initial load.

A client with `#[lazy]` code and `split = false` now fails to build with the fix, instead of shipping a page whose loader import cannot resolve. A split build with `RUSTFLAGS` set must include `--emit-relocs` in it.

`leptoskit_cli` now needs Rust 1.95 or newer, for the `if let` guards wasm-split uses. In the `islands` example, splitting off one lazy island cuts the initial load from 87.5 KiB to 71.2 KiB brotli-compressed.
