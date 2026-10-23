<p align="center">
  <a href="https://ifiokjr.github.io/leptoskit/">
    <picture>
      <source media="(prefers-color-scheme: dark)" srcset="docs/src/assets/logo-dark.svg">
      <img src="docs/src/assets/logo-light.svg" alt="leptoskit" width="360">
    </picture>
  </a>
</p>

<p align="center">
  <strong>One leptos tree. Real native widgets. Updated over the air.</strong>
</p>

<p align="center">
  <a href="https://ifiokjr.github.io/leptoskit/">Documentation</a> ·
  <a href="https://crates.io/crates/leptoskit">crates.io</a> ·
  <a href="https://docs.rs/leptoskit">docs.rs</a> ·
  <a href="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml">CI</a>
</p>

---

leptoskit is one toolkit for every place a leptos app runs. Write leptos components and ship them to the browser with SSR and hydration. Ship the same interface as **real native widgets** on iOS, Android, and the desktop, with no web view, and push interface updates over the air the way React Native does.

The logo is the architecture: the ember boxes are layout, elements placed by the layout engine and committed as native widgets. The mint piece is the next version of your interface, arriving over the air.

## Examples

Every screen here is a leptoskit element tree committed as real AppKit views by [`leptoskit_appkit`](crates/leptoskit_appkit), then rendered offscreen at 2x. The phone screens are drawn on macOS at a phone-sized viewport. Captions and the dashboard are on the [examples page](https://ifiokjr.github.io/leptoskit/examples.html), and `cargo run -p gallery -- snapshot <dir>` regenerates them.

<p align="center">
  <img src="https://f002.backblazeb2.com/file/leptoskit/examples/macos/profile.png" alt="A profile screen" width="200">
  <img src="https://f002.backblazeb2.com/file/leptoskit/examples/macos/settings.png" alt="A settings screen" width="200">
  <img src="https://f002.backblazeb2.com/file/leptoskit/examples/macos/chat.png" alt="A chat screen" width="200">
</p>

## The platforms

Every platform has a guide in [the documentation](https://ifiokjr.github.io/leptoskit/platforms/readme.html): [web](docs/src/platforms/web.md), [iOS](docs/src/platforms/ios.md), [Android](docs/src/platforms/android.md), [macOS](docs/src/platforms/macos.md), and [Windows/Linux](docs/src/platforms/windows-linux.md), plus [code reuse across platforms](docs/src/platforms/code-reuse.md).

The [`platform_probe`](examples/platform_probe) suite tests the whole stack: yoga layout, renderer diffing, the wasm ABI runtime, and a code push delta update over the real HTTP surface. CI runs it on Linux, macOS, Windows, and the iOS simulator, and cross-compiles it for Android. Locally, run it with `test:all`, `ios_sim_run`, or `android_emulator_run`.

Platform backends apply mutation batches to UIKit, AppKit, Android, Win32, and GTK widgets. AppKit is the most complete; the others render every built-in primitive and run in CI on a simulator, an emulator, or a desktop runner. The renderer also has a headless backend for portable tests.

## The architecture

| Crate                                                     | What it is                                                                                                                                                                   |
| --------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| [`leptoskit_yoga`](crates/leptoskit_yoga)                 | A from-scratch layout engine: flexbox **and** CSS grid (`fr`, `minmax`, spans, dense auto-placement), gaps, `aspect_ratio`, RTL, `display: contents`.                        |
| [`leptoskit_native`](crates/leptoskit_native)             | The native renderer, built on the React Native renderer's design: element trees diff into mutation batches that a platform backend applies to real widgets.                  |
| [`leptoskit_appkit`](crates/leptoskit_appkit)             | The macOS backend: commits mutation batches into real AppKit views, measures text with AppKit's own font metrics, and renders view trees to PNG offscreen.                   |
| [`leptoskit_wasm`](crates/leptoskit_wasm)                 | The on-device runtime: runs the app's wasm bundle in `wasmi`, an interpreter, so it is allowed on iOS, and drives the renderer through a versioned, fuel-metered ABI.        |
| [`leptoskit_codepush`](crates/leptoskit_codepush)         | Over-the-air updates: content-hashed manifests, a single-file bundle container, binary deltas, an update protocol, Axum routes, and a client that verifies every hash.       |
| [`leptoskit_axum`](crates/leptoskit_axum)                 | The Axum server toolkit: static files with precompressed gzip, SPA fallback, health checks, and the code push registry. Axum is the one supported server.                    |
| [`leptoskit_rpc`](crates/leptoskit_rpc)                   | Retrying queries and mutations over leptos server functions, with errors classified so "offline" and "invalid input" stop looking the same.                                  |
| [`leptoskit_router`](crates/leptoskit_router)             | Typed file routes: `src/routes/` becomes one `Route` enum that parses URLs and writes hrefs, generated by `leptoskit routes` with `leptoskit_router_codegen`.                |
| [`leptoskit_query`](crates/leptoskit_query)               | Async state in the style of TanStack Query: cached, deduplicated, retrying queries, infinite queries, and mutations on fine-grained signals, synced across tabs and windows. |
| [`leptoskit_use`](crates/leptoskit_use)                   | Cross-platform reactive hooks: state, derived values, watchers, timers, storage, and viewport breakpoints without a DOM or executor.                                         |
| [`leptoskit_config`](crates/leptoskit_config)             | `leptoskit.toml`: app identity, platforms, servers, native targets, code push channels, and plugin declarations.                                                             |
| [`leptoskit_plugin`](crates/leptoskit_plugin)             | The plugin system: build hooks for leptoskit plugins, and leptos plugins forwarded as app cargo features.                                                                    |
| [`leptoskit_docs`](crates/leptoskit_docs)                 | Markdown with leptos components, in the style of MDX, and the static site generator that builds these docs: navigation, search, highlighting, and checked links.             |
| [`leptoskit_cli`](crates/leptoskit_cli)                   | The `leptoskit` binary: `new`, `dev`, `build`, `serve`, `codepush`, `doctor`, `completions`.                                                                                 |
| [`leptos_testing_library`](crates/leptos_testing_library) | Testing Library style DOM queries for leptos components, run in a real browser with `wasm-bindgen-test`.                                                                     |
| [`leptoskit_testing`](crates/leptoskit_testing)           | Native widget tests with semantic finders, interactions, bounded pumping, and diagnostic trees.                                                                              |
| [`leptoskit`](crates/leptoskit)                           | The meta crate that re-exports the toolkit.                                                                                                                                  |

## The CLI

```bash
cargo install leptoskit_cli --locked

leptoskit new my_app                     # one app, all six platforms enabled
leptoskit dev                            # watch, rebuild, serve
leptoskit routes                         # write the typed Route enum from src/routes/
leptoskit build --release --platform web # cached builds
leptoskit serve                          # static files, SPA fallback, health check
leptoskit codepush release               # pack dist/ into a versioned release
leptoskit codepush publish               # upload releases to an S3-compatible bucket
leptoskit codepush serve                 # or serve updates to devices yourself
leptoskit doctor                         # check the toolchain
```

Both installs include `lk`, a shorter alias for the same CLI: `lk new my_app`, `lk dev`, `lk build`. Help and completions answer to whichever name you type.

`leptoskit new amazing` creates `amazing_app`, `amazing_backend`, and `amazing_shared` crates, with the SSR server in root `src/main.rs`. The app owns routes, components, and state for every platform. Backend implementation stays server-only, and shared contracts live in the shared crate. App-owned `web/`, `ios/`, `android/`, `macos/`, `windows/`, and `linux/` folders contain platform integration. Generated state stays in ignored `.leptoskit/` directories.

Each build selects one platform feature. Run `leptoskit build --platform web`, `leptoskit build --platform macos`, or `leptoskit native build ios-simulator`. Use `leptoskit::platform_module!` to replace a shared module with a `.<platform>.rs` file while keeping its imports. See [creating a project](docs/src/guide/02-creating.md) for dependencies and overrides.

Speed is a feature. cargo stays in charge of compiling, and leptoskit fingerprints the steps after it, so wasm-bindgen and `wasm-opt` only run again when the compiled wasm actually changed. The fingerprints live in `target/leptoskit/cache/`.

## Native is really native

A leptos component tree maps one-to-one onto native widgets: every `Element` becomes a `UIView`, an Android view, or an `NSView` through the platform backend, never a web view. The wasm bundle running that interface is the same artifact code push delivers, so an app shell that went through store review can run any interface version the server has.

## Server functions

[`leptoskit_rpc`](crates/leptoskit_rpc) wraps leptos server functions in retrying, error-classified queries and mutations. The same calls run from the browser, from native shells, and during SSR.

## Contributing

[`devenv`](https://devenv.sh/) provides the reproducible development environment.

```bash
devenv shell install:all   # tools and dependencies
devenv shell test:all      # nextest + doctests
devenv shell lint:all      # clippy -D warnings, leptoskit format, dprint, cargo-deny
devenv shell fix:all       # autofix everything fixable
devenv shell docs:serve    # the documentation site, with live reload
```

Editor settings live in `setup/editors`. Run `setup:vscode` or `setup:helix` to copy them into place.

## Releases

Releases run through [monochange](https://crates.io/crates/monochange) changesets:

1. A feature branch carries a changeset: `monochange run change --package leptoskit --bump patch --reason "..."`.
2. The `release` workflow keeps one release pull request open that summarises the pending changesets. Merging it cuts the release.
3. The merge pushes the tag, drafts the GitHub release, and dispatches the `publish` workflow, which publishes every crate to crates.io in dependency order. It uses trusted publishing when the OIDC identity verifies, and falls back to `CARGO_REGISTRY_TOKEN` for a crate's first publish, before its trusted publisher exists.

Every crate releases in lockstep as one version group under a single tag. Each keeps its own `changelog.md`, and the root carries the combined summary.

## License

[Unlicense](license)
