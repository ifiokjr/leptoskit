# Building and serving

## `leptoskit dev`

One interface on six platforms means six places a change can look wrong. `leptoskit dev` runs all of them at once and keeps them current: save a file, and the browser tab, the iOS Simulator, the Android emulator, and the desktop window all show the change.

```bash
leptoskit dev
leptoskit dev --platform ios --platform android
leptoskit dev --open --port 8080
leptoskit dev --platform tui
```

With no `--platform`, `dev` runs every platform in `app.platforms` that this machine can run, and says which it skipped and why. On a Mac with Xcode and the Android SDK installed, a new project runs on the web, iOS, Android, and macOS, and skips Windows and Linux. `--platform` runs only the platforms you name, and fails if this machine can't run one of them.

| Platform | Where it runs                                                                     | When you save                                               |
| -------- | --------------------------------------------------------------------------------- | ----------------------------------------------------------- |
| Web      | `dist/`, served on `serve.address` and `serve.port`                               | The site rebuilds. A server-rendered app's server restarts. |
| iOS      | Every booted iOS Simulator. With none booted, the newest iPhone simulator boots.  | The new interface replaces the old one in the running app.  |
| Android  | Every attached device or emulator. With none attached, your first emulator boots. | The new interface replaces the old one in the running app.  |
| macOS    | This Mac.                                                                         | The new interface replaces the old one in the same window.  |
| Windows  | This PC, when you run `dev` on Windows.                                           | The new interface replaces the old one in the same window.  |
| Linux    | This machine, when you run `dev` on Linux.                                        | The new interface replaces the old one in the same window.  |
| Terminal | This terminal, on its own.                                                        | Nothing: quit with `Ctrl+C` and run `dev` again.            |

Each native app is built and installed once, when `dev` starts. After that, only your interface rebuilds: `dev` compiles the WebAssembly each app runs and sends it to the apps, which swap it in without being reinstalled, so the app keeps its window and its unchanged controls. Your app's state starts over with each change. If the new interface fails to start, the app keeps showing the last one that worked and logs the error, and the next save tries again.

The apps' logs appear in the same terminal, each line marked with its platform, such as `[ios]` or `[android]`. Changes to `leptoskit.toml` reach the interface, but plugins and other app settings need the apps rebuilt, so restart `dev` after changing them. iOS runs in the simulator only; to run on an iPhone, build it with `leptoskit build --platform ios` and install it from Xcode.

`dev` also starts the DevTools hub, which shows the element tree, styles, and data of every app it runs, on every platform at once: see [Debugging with DevTools](26-devtools.md).

Ctrl-C stops `dev` along with your app's server and the desktop apps it opened. Simulators and emulators keep running, so the next `dev` finds them already booted.

The [terminal app](../platforms/tui.md) needs the terminal you run `dev` in, so it never runs beside the other platforms. A plain `dev` skips it unless `app.default_platform` is `tui`, and `--platform tui` must be the only platform you name. `dev` builds it once and runs it until you quit.

On the web, `app.render = "csr"` serves `dist/` itself. With `"ssr"` or `"islands"`, `dev` runs your server binary instead, with the `LEPTOS_*` environment pointing at `dist/`. New projects render on the server, so `dev` builds and runs your server. `dev` watches workspace sources, configuration, and web assets, and waits for a burst of changes to finish before rebuilding.

| Flag                    | Default                                    | Meaning                                                                                                                                     |
| ----------------------- | ------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------- |
| `--platform <PLATFORM>` | every platform this machine can            | A platform to run. Repeat it for several; `tui` runs alone.                                                                                 |
| `--device <DEVICE>`     | every booted simulator and attached device | Runs iOS and Android only on this simulator (name or UDID) or adb serial. Repeat it for several; a named simulator that is shut down boots. |
| `--port <PORT>`         | `serve.port` (`3000`)                      | The port to serve the web app on.                                                                                                           |
| `--open`                | off                                        | Opens the web app in your browser.                                                                                                          |

It checks it can bind `serve.address` before the first build, so a port that's already taken fails straight away instead of after a build.

Only debug builds accept an interface from `dev`, because those interfaces are not signed the way [code push](05-code-push.md) releases are. A release build ignores it, so never ship a debug build.

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

With `app.render = "ssr"` or `"islands"`, `serve` runs the server binary the last `leptoskit build` produced, with leptos in its `PROD` environment and pointed at `--dir`. Your server renders the pages and serves the files; mount [`static_files`](https://docs.rs/leptoskit_axum/latest/leptoskit_axum/fn.static_files.html) from `leptoskit::server` as its fallback to serve `dist/` the same way leptoskit does.

With `app.render = "csr"`, `serve` runs the same Axum server your production deployment can use:

- **Static files**, with the precompressed `.br` or `.gz` copy when the browser accepts brotli or gzip, and `Vary: Accept-Encoding` so caches keep them apart.
- **SPA fallback**: `GET` and `HEAD` requests for a path whose last segment has no file extension get `index.html`. Anything else that doesn't exist, such as a missing `.wasm` file, gets a real 404.
- **A health check** at `/_leptoskit/health`.

To use the same server from your own binary, depend on `leptoskit` with its `server` feature, described next. Before either server answers the internet, read [security and scale](16-security-and-scale.md).

## One dependency for the server

A server that renders pages or answers server functions is built from a handful of crates: `axum`, `tokio`, `tower`, `tower-http`, `leptos`, and `leptos_axum`. Every leptoskit deployment links them, so `leptoskit` re-exports them, and a server package depends on `leptoskit` alone:

```toml
[dependencies]
leptoskit = { version = "0.1", features = ["server"] }
```

- **`leptoskit::server`** is the server toolkit (`serve`, `serve_router`, `serve_listener`, `static_files`, `dev_router`) together with `axum`, `tokio`, `tower`, `tower_http`, and `leptos_axum`, at the versions leptoskit was built against. `tokio` carries the `macros` and `rt-multi-thread` features, so `use leptoskit::server::tokio;` is all `#[tokio::main]` needs; `tower` and `tower_http` carry the limit, load-shed, and timeout layers from [security and scale](16-security-and-scale.md).
- **`leptoskit::leptos`**, **`leptoskit::leptos_router`**, and **`leptoskit::leptos_meta`** are leptos itself, behind the `leptos` feature. The `csr`, `hydrate`, `ssr`, and `islands` features forward to them, so a package can write `hydrate = ["leptoskit/hydrate"]` instead of naming leptos, and `leptoskit build` turns islands on through `leptoskit/islands` when a package reaches leptos this way. `server` implies `ssr`.
- **`leptoskit::codepush`** re-exports `ed25519_dalek` and `semver`, for code that signs or numbers releases itself rather than through the CLI.

The one thing that stays a dependency of its own is `leptos` in a crate that uses its macros: `view!`, `#[component]`, `#[island]`, and `#[server]` expand to `::leptos::` paths, which only a direct dependency resolves. `leptoskit new` follows that line: the `_app` crate depends on `leptos`, while the server and backend packages depend on `leptoskit` alone. This is the server it generates:

```rust,ignore
use leptoskit::server::leptos_axum::LeptosRoutes;
use leptoskit::server::leptos_axum::generate_route_list;
use leptoskit::server::tokio;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
	let options = leptoskit::leptos::config::get_configuration(None)?.leptos_options;
	let address = options.site_addr;
	let package_dir = format!("{}/{}", options.site_root, options.site_pkg_dir);
	let routes = generate_route_list(App);
	let app_options = options.clone();
	let app = my_app_backend::router()
		.nest_service("/pkg", leptoskit::server::static_files(package_dir.as_ref()))
		.leptos_routes(&options, routes, move || shell(app_options.clone()))
		.with_state(options);
	let listener = tokio::net::TcpListener::bind(address).await?;
	println!("listening on http://{address}");
	leptoskit::server::serve_listener(listener, app).await?;
	Ok(())
}
```

`serve_listener` and `serve_router` finish the requests in flight on Ctrl-C or `SIGTERM`, the same way `leptoskit serve` does, and `static_files` serves the precompressed copies a release build writes.
