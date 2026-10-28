# Building and serving

## `leptoskit dev`

```bash
leptoskit dev
leptoskit dev --open --port 8080
leptoskit dev --platform tui
```

`dev` makes a debug build into `dist/`, serves it, and rebuilds when source files change. With `app.render = "csr"` it serves `dist/` itself. With `"ssr"` or `"islands"` it runs your server binary instead, with the `LEPTOS_*` environment pointing at `dist/`, and restarts it after every successful rebuild. It watches workspace sources, configuration, and web assets, and waits for a burst of changes to finish before rebuilding.

New projects render on the server (`app.render = "ssr"`), so `dev` builds and runs your server. A client-rendered app needs no server package of its own: leptoskit serves `dist/`, and the page comes from `web/index.html`.

`--platform tui` builds your [terminal app](../platforms/tui.md) and runs it in the terminal you typed the command in; quit with `Ctrl+C` and run it again after a change. For the other native platforms, run `leptoskit build --platform <platform>` after each change.

| Flag                    | Default                | Meaning                                                  |
| ----------------------- | ---------------------- | -------------------------------------------------------- |
| `--port <PORT>`         | `serve.port` (`3000`)  | The port to serve on.                                    |
| `--open`                | off                    | Opens the app in your browser.                           |
| `--platform <PLATFORM>` | `app.default_platform` | `web` (or `server`) serves the site; `tui` runs the app. |

It checks it can bind `serve.address` before the first build, so a port that's already taken fails straight away instead of after a build.

## `leptoskit build`

```bash
leptoskit build
leptoskit build --release --platform web --out dist
leptoskit build --platform macos
leptoskit native build ios-simulator
```

| Flag                          | Default                | Meaning                                      |
| ----------------------------- | ---------------------- | -------------------------------------------- |
| `--release`                   | off                    | An optimised build, with `wasm-opt`.         |
| `--platform <PLATFORM>`       | `app.default_platform` | The platform to build, from `app.platforms`. |
| `--optimize <fast\|smallest>` | `bundle.optimize`      | How hard to optimize a release client.       |
| `--out <DIR>`                 | `dist`                 | Where the built web site goes.               |

The platforms are `web`, `ios`, `android`, `macos`, `windows`, and `linux`, plus the experimental `harmonyos`. A native build compiles your app to WebAssembly, runs the platform's own build tools (Xcode, Gradle, Cargo), and writes into `.leptoskit/<platform>/`. `leptoskit native build` also takes simulator and architecture variants such as `ios-simulator` and `android-x86_64`. See [platforms](../platforms/readme.md) for what each needs. The rest of this section is about web builds.

A web build:

1. compiles the server package named by `app.name`, and the `cdylib` client's library for `wasm32-unknown-unknown`;
2. when the client contains lazy code, runs wasm-split in process by default, writing the lazily loaded chunks, their loader, and `__wasm_split_manifest.json` to `<out>/pkg/` (`bundle.split = false` opts out);
3. runs wasm-bindgen in process, writing `<out>/pkg/<client>.js` and `<out>/pkg/<client>_bg.wasm`;
4. runs `wasm-opt -Oz` over the module and every chunk on release builds, when `bundle.wasm_opt` is on;
5. copies `web/` into `<out>` and shared `assets/` into `<out>/assets/`, also accepting `public/` from older projects;
6. on release builds, writes `.br` and `.gz` copies of every compressible file, and prints the [client bundle report](09-small-bundles.md#see-what-you-ship).

`bundle.optimize` decides how hard a release build optimizes the wasm client, and `--optimize fast` or `--optimize smallest` overrides it for one build (see [release builds](09-small-bundles.md#release-builds)). The server keeps your own `release` profile either way.

`--platform server` compiles only the web server. Older projects can also use `--platform native`, which builds the platform in `native.targets`; projects created with `leptoskit new` name the platform instead.

Features from `build.features`, and from any leptos plugins you've turned on, reach both cargo builds. `app.render` adds each side's own features on top: `ssr` for the server and `hydrate` for the client, plus `leptos/islands` for both in islands mode (see [render modes](03-configuration.md#render-modes)).

In a project created with `leptoskit new`, each build turns on exactly one platform feature in `<name>_app`: `web` for the browser, `web` and `ssr` for the server, or one native platform plus `native` for a native app. The browser and each native platform build into their own target directory under `.leptoskit/<platform>/`, so switching platforms doesn't throw away the other builds.

The server and the client can be one package, the layout most leptos apps use: a crate with both `src/main.rs` and a `cdylib` library. When the package `app.name` names has a `cdylib` target, it is the client too. The wasm build compiles only its library, so the server code never has to build for wasm. `build.rustflags` applies to the wasm client and merges with `[target.wasm32-unknown-unknown] rustflags` in `.cargo/config.toml`. Once it is set, cargo ignores a `[build] rustflags` table for the wasm build, so keep wasm flags under the target table.

### What gets cached

Cargo is already incremental, so leptoskit runs it every time and lets it decide what to recompile. leptoskit caches the step after it: wasm-bindgen and `wasm-opt` run again only when the compiled wasm, the build mode, the output directory, or the `wasm-opt` version has changed, or when their outputs are missing. Fingerprints live in `<target dir>/leptoskit/cache/`, one file per phase, so projects and builds sharing a target directory never overwrite each other.

Before binding, `build` checks that your `Cargo.lock` pins the same `wasm-bindgen` the CLI embeds. If it doesn't, the error includes the `cargo update --precise` command that fixes it.

> **Note:** `bundle.wasm_opt` is on by default, so a release build fails with an install hint when `wasm-opt` isn't on your `PATH`. Install binaryen, or set `wasm_opt = false` under `[bundle]`.

## `leptoskit serve`

```bash
leptoskit serve
leptoskit serve --dir dist --port 8080
```

| Flag            | Default      | Meaning                 |
| --------------- | ------------ | ----------------------- |
| `--dir <DIR>`   | `dist`       | The directory to serve. |
| `--port <PORT>` | `serve.port` | The port to serve on.   |

With `app.render = "ssr"` or `"islands"`, `serve` runs the server binary the last `leptoskit build` produced, with leptos in its `PROD` environment and pointed at `--dir`. Your server renders the pages and serves the files; mount [`leptoskit_axum::static_files`](https://docs.rs/leptoskit_axum/latest/leptoskit_axum/fn.static_files.html) as its fallback to serve `dist/` the same way leptoskit does.

With `app.render = "csr"`, `serve` runs the same Axum server your production deployment can use:

- **Static files**, with the precompressed `.br` or `.gz` copy when the browser accepts brotli or gzip, and `Vary: Accept-Encoding` so caches keep them apart.
- **SPA fallback**: `GET` and `HEAD` requests for a path whose last segment has no file extension get `index.html`. Anything else that doesn't exist, such as a missing `.wasm` file, gets a real 404.
- **A health check** at `/_leptoskit/health`.

To use the same server from your own binary, depend on [`leptoskit_axum`](https://docs.rs/leptoskit_axum). Before either server answers the internet, read [security and scale](16-security-and-scale.md).
