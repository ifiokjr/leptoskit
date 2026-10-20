<p align="center">
  <a href="https://ifiokjr.github.io/leptoskit/"><img src="https://raw.githubusercontent.com/ifiokjr/leptoskit/main/docs/src/assets/mark.svg" alt="leptoskit" width="64"></a>
</p>

# leptoskit_cli

> The `leptoskit` command line interface.

```bash
leptoskit new my_app --platform native
leptoskit dev            # build, serve, rebuild on change
leptoskit build --release --platform web
leptoskit serve
leptoskit codepush release
leptoskit codepush serve
leptoskit codepush check
leptoskit doctor
leptoskit test run --binary target/debug/examples/widget_suite
leptoskit test web --package leptos_testing_library --browser chrome
leptoskit completions zsh
```

`lk` is a shorter alias for the same CLI — every command works identically as `lk new`, `lk dev`, `lk build`, and completions answer to the invoked name (`lk completions zsh > ~/.zfunc/_lk`).

`test run` executes a prebuilt native test binary on the host, a selected iOS simulator, or an Android device/emulator. `test web` runs browser component tests through wasm-bindgen-test. Both retain output and JSON reports. See the [testing guide](https://github.com/ifiokjr/leptoskit/blob/main/docs/src/guide/07-testing.md) for device startup, deadlines, artifacts, and current platform limits.

`leptoskit new <dir>` writes a small cargo workspace that `leptoskit build` compiles straight away: a server binary named after the app at the root, a `cdylib` wasm client in `client/` pinned to the wasm-bindgen this CLI embeds, and a `public/index.html` that loads the bound client. It never overwrites an existing file.

## It is also the cargo-leptos replacement

`leptoskit build` runs the whole web pipeline cargo-leptos runs — server compile, wasm client compile, wasm-bindgen glue, optional `wasm-opt`, asset layout:

- **Cargo always runs.** Server first, then the wasm client. Both share cargo's target directory (and its lock), and cargo's own incremental cache makes an unchanged build a sub-second no-op — and it is the only thing that sees a changed dependency, build script, or environment variable.
- **In-process wasm-bindgen.** The glue generation runs as a library call, not a CLI subprocess, and is keyed on the compiled wasm's _content hash_ plus the output directory and flags, recorded in `<target dir>/leptoskit/cache.json`. An unchanged compile into the same place skips bindgen entirely; deleted outputs or a new `--out` directory rebind.
- **A version check before binding.** wasm-bindgen only accepts wasm built against its own version, so `build` (and `doctor`) compare the project's locked `wasm-bindgen` with the embedded one and fail with the `cargo update` command that fixes a mismatch.
- **No optimizer in dev.** `wasm-opt` only ever runs in release (unless `build.wasm_opt = false`), as part of the cached bindgen phase.
- **Content-compared assets.** The `public/` tree hashes with rayon and copies only files whose content differs from the copy already in the output.

`build.features`, leptos plugins (`leptos = true`), and plugin features reach both cargo builds as `--features`; plugin environment variables are set on both. `build.rustflags` and plugin rustflags apply to the wasm client only, passed as `--config target.wasm32-unknown-unknown.rustflags=[...]` so they join any rustflags in `.cargo/config.toml` (an exported `RUSTFLAGS` overrides them, as it does every cargo config). An unknown plugin name is an error.

The client crate is detected automatically — the workspace member producing a `cdylib` target — or pinned with `build.client_package` in `leptoskit.toml`. The dev loop binds `serve.address`, watches every workspace member, `Cargo.toml`, `leptoskit.toml`, and `public/`, and rebuilds once changes have been quiet for 150 ms.
