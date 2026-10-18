# leptoskit_cli

> The `leptoskit` command line interface.

```bash
leptoskit new my_app --platform native
leptoskit dev            # watch, rebuild, serve
leptoskit build --release --platform web
leptoskit serve
leptoskit codepush release
leptoskit codepush serve
leptoskit codepush check
leptoskit doctor
leptoskit completions zsh
```

## It is also the cargo-leptos replacement

`leptoskit build` runs the whole web pipeline cargo-leptos runs — server compile, wasm client compile, wasm-bindgen glue, optional `wasm-opt`, asset layout — but organized for speed the way the Dioxus CLI is:

- **Parallel compiles.** The server and the wasm client build on separate threads, so the wall time is the slower of the two, not the sum.
- **In-process wasm-bindgen.** The glue generation runs as a library call, not a CLI subprocess, and is keyed on the compiled wasm's _content hash_: an unchanged compile skips bindgen entirely, which removes the biggest fixed cost of a typical rebuild.
- **Phase fingerprints.** Every phase (cargo server, cargo wasm, bindgen, wasm-opt, asset copy) records a content-hash + tool-version fingerprint in `target/leptoskit/cache.json`; a build after a one-line change runs only the phases whose inputs moved.
- **No optimizer in dev.** `wasm-opt` only ever runs in release, and only when its input moved.
- **Parallel asset hashing.** The public tree hashes with rayon and copies only files whose content changed.

The client crate is detected automatically — the workspace member producing a `cdylib` target — or pinned with `build.client_package` in `leptoskit.toml`. The dev loop layers a debouncing watcher over the same cached pipeline, so rebuilds after editor bursts touch only what moved.
