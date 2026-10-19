# leptoskit

> The all-encompassing shop for building and deploying leptos apps: web, native, and over the air.

leptoskit is one toolkit for every place a leptos app runs. Write leptos components; ship them to the browser with SSR and hydration; ship them as **real native widgets** on iOS, Android, and desktop — no web view — and push interface updates over the air the way React Native does.

## The platforms

Every platform has a guide under [`docs/platforms/`](docs/platforms) — [web](docs/platforms/web.md), [iOS](docs/platforms/ios.md), [Android](docs/platforms/android.md), [macOS](docs/platforms/macos.md), and [Windows/Linux](docs/platforms/windows-linux.md) — plus [code reuse across platforms](docs/platforms/code-reuse.md). The [`platform_probe`](examples/platform_probe) suite validates the whole stack (yoga layout, renderer diffing, the wasm ABI runtime, and a code push delta update over the real HTTP surface) on the desktop via `test:all`, on the iOS simulator via `ios_sim_run`, and on Android via `android_emulator_run`.

## The architecture

| Crate                                             | What it is                                                                                                                                                                                               |
| ------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| [`leptoskit_yoga`](crates/leptoskit_yoga)         | A from-scratch layout engine: flexbox **and** CSS grid (`fr`, `minmax`, spans, dense auto-placement), gaps, `aspect_ratio`, RTL, `display: contents`. One arena, zero pointers, cached text measurement. |
| [`leptoskit_native`](crates/leptoskit_native)     | The native renderer, rewritten from the React Native renderer's design: element trees diff into mutation batches that a platform backend applies to real widgets. Headless backend included.             |
| [`leptoskit_wasm`](crates/leptoskit_wasm)         | The on-device runtime: runs the app's wasm bundle in `wasmi` (an interpreter, so it is iOS-legal) and drives the renderer through a versioned, fuel-metered ABI.                                         |
| [`leptoskit_codepush`](crates/leptoskit_codepush) | Over-the-air updates: content-hashed manifests, a single-file bundle container, binary deltas between versions, an update protocol, Axum routes, and a client that verifies every hash.                  |
| [`leptoskit_axum`](crates/leptoskit_axum)         | The Axum server toolkit: static serving with pre-compressed gzip, SPA fallback, health checks, and the code push registry. Axum is the one supported server.                                             |
| [`leptoskit_fns`](crates/leptoskit_fns)           | The nice server-fn API: retrying queries and mutations over leptos server functions, with errors classified so "offline" and "invalid input" stop looking the same.                                      |
| [`leptoskit_config`](crates/leptoskit_config)     | `leptoskit.toml`: app identity, platforms, servers, native targets, code push channels, and plugin declarations.                                                                                         |
| [`leptoskit_plugin`](crates/leptoskit_plugin)     | The plugin system: build hooks for leptoskit plugins, and full leptos plugin support forwarded as app cargo features.                                                                                    |
| [`leptoskit_cli`](crates/leptoskit_cli)           | The `leptoskit` binary (crate `leptoskit_cli`): `new`, `dev`, `build`, `serve`, `codepush`, `doctor`, `completions` — with content-addressed phase caching so rebuilds skip everything unchanged.        |
| [`leptoskit`](crates/leptoskit)                   | The meta crate re-exporting all of it.                                                                                                                                                                   |

## The CLI

```bash
leptoskit new my_app --platform native   # scaffold a project
leptoskit dev                            # watch, rebuild, serve
leptoskit build --release --platform web # cached, parallel builds
leptoskit serve                          # static + SPA + health
leptoskit codepush release               # pack dist/ into a versioned release
leptoskit codepush serve                 # serve updates to devices
leptoskit doctor                         # check the toolchain
```

Speed is a feature: every pipeline phase fingerprints its inputs (content hashes plus tool versions) into `target/leptoskit/cache.json`, so a build after a one-line change runs only the phases whose inputs moved — cargo, wasm-bindgen, and asset copies each skip independently, and assets hash in parallel with rayon.

## Native is really native

A leptos component tree has a one-to-one representation in native widgets: every `Element` becomes a `UIView`, an Android view, or an `NSView` through the platform backend — never an optimized web view. The wasm bundle running that interface is the same artifact code push delivers, so a store-submitted shell can serve any interface version the server has.

## Server functions

Axum-only, with [`leptoskit_fns`](crates/leptoskit_fns) wrapping leptos server functions in retrying, error-classified queries and mutations. The same calls run from the browser, from native shells, and from SSR.

## Releases

Releases run through [monochange](https://crates.io/crates/monochange) with changesets, the same process as every other Rust workspace here:

1. A feature branch carries a changeset: `monochange run change --package leptoskit --bump patch --reason "..."`
2. The `release` workflow keeps one release pull request open summarizing the pending changesets; merging it cuts the release.
3. The merge pushes the tag, drafts the GitHub release, and dispatches the `publish` workflow, which publishes every crate to crates.io in dependency order — with trusted publishing when the OIDC identity verifies, falling back to `CARGO_REGISTRY_TOKEN` for the first publish of a crate before its trusted-publisher enrollment exists.

All eleven crates release in lockstep as one version group with a single tag; each keeps its own `changelog.md` and the root carries the combined summary.

## Contributing

[`devenv`](https://devenv.sh/) provides the reproducible development environment.

```bash
devenv shell install:all   # tools and dependencies
devenv shell test:all      # nextest + doctests
devenv shell lint:all      # clippy -D warnings, dprint, zizmor, cargo-deny
devenv shell fix:all       # autofix everything fixable
```

To set up recommended editor configurations:

```bash
setup:vscode # Setup vscode
setup:helix  # Setup helix configuration
```
