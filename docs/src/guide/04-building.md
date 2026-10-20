# Building and serving

## `leptoskit dev`

```bash
leptoskit dev
leptoskit dev --open --port 8080
```

`dev` makes a debug build into `dist/`, serves it, and rebuilds when anything changes. It watches every workspace member, the root `Cargo.toml`, `leptoskit.toml`, and `public/`, and waits for a burst of changes to finish before rebuilding.

| Flag            | Default               | Meaning                        |
| --------------- | --------------------- | ------------------------------ |
| `--port <PORT>` | `serve.port` (`3000`) | The port to serve on.          |
| `--open`        | off                   | Opens the app in your browser. |

It binds `serve.address` before the first build, so a port that's already taken fails straight away instead of after a build.

## `leptoskit build`

```bash
leptoskit build
leptoskit build --release --platform web --out dist
```

| Flag                               | Default                | Meaning                              |
| ---------------------------------- | ---------------------- | ------------------------------------ |
| `--release`                        | off                    | An optimised build, with `wasm-opt`. |
| `--platform <web\|native\|server>` | `app.default_platform` | What to build.                       |
| `--out <DIR>`                      | `dist`                 | Where the built site goes.           |

A web build:

1. compiles the server package named by `app.name`, and the `cdylib` client for `wasm32-unknown-unknown`;
2. runs wasm-bindgen in process, writing `<out>/pkg/<client>.js` and `<out>/pkg/<client>_bg.wasm`;
3. runs `wasm-opt` on release builds, when `build.wasm_opt` is on;
4. copies `public/` into `<out>`.

A `server` build compiles only the server. A `native` build does everything except copy `public/`.

Features from `build.features`, and from any leptos plugins you've turned on, reach both cargo builds. `build.rustflags` applies to the wasm client and merges with `[target.wasm32-unknown-unknown] rustflags` in `.cargo/config.toml`. Once it is set, cargo ignores a `[build] rustflags` table for the wasm build, so keep wasm flags under the target table.

### What gets cached

Cargo is already incremental, so leptoskit runs it every time and lets it decide what to recompile. leptoskit caches the step after it: wasm-bindgen and `wasm-opt` run again only when the compiled wasm, the build mode, the output directory, or the `wasm-opt` version has changed, or when their outputs are missing. Fingerprints live in `<target dir>/leptoskit/cache.json`.

Before binding, `build` checks that your `Cargo.lock` pins the same `wasm-bindgen` the CLI embeds. If it doesn't, the error includes the `cargo update --precise` command that fixes it.

> **Note:** `build.wasm_opt` is on by default, so a release build fails with an install hint when `wasm-opt` isn't on your `PATH`. Install binaryen, or set `wasm_opt = false` under `[build]`.

## `leptoskit serve`

```bash
leptoskit serve
leptoskit serve --dir dist --port 8080
```

| Flag            | Default      | Meaning                 |
| --------------- | ------------ | ----------------------- |
| `--dir <DIR>`   | `dist`       | The directory to serve. |
| `--port <PORT>` | `serve.port` | The port to serve on.   |

`serve` runs the same Axum server your production deployment can use:

- **Static files**, with precompressed `.gz` files when the browser accepts gzip, and `Vary: Accept-Encoding` so caches keep the two apart.
- **SPA fallback**: `GET` and `HEAD` requests for a path whose last segment has no file extension get `index.html`. Anything else that doesn't exist, such as a missing `.wasm` file, gets a real 404.
- **A health check** at `/_leptoskit/health`.

To use the same server from your own binary, depend on [`leptoskit_axum`](https://docs.rs/leptoskit_axum).
