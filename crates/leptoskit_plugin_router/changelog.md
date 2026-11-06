# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.0](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.0) (2026-11-06)

### Breaking changes

#### leptoskit_plugin_router: file routes as a build plugin

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_router` is the file router as a leptoskit plugin: it generates the typed `Route` module from the files under `src/routes/` before cargo runs, and answers `plugin:router routes`.

Turn it on with a `[[plugins]]` entry named `router` in `leptoskit.toml`; `leptoskit dev`, `build`, and `native` then rewrite `src/routes.rs` only when the routes change, and `leptoskit plugin:router routes --check` fails when the committed module is stale. Apps don't depend on this crate, since the CLI links it; the runtime and the `Route` derive live in `leptoskit_router`.

### Fixes

- **Correct README claims about generated renderers, accessibility, runtime plugin registration, password screens, RPC macros, and interpreter updates.** Share crate summaries with Rust documentation and derive MSRV badges from Cargo through MDT templates, with a documentation drift check. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #515](https://github.com/leptoskit/leptoskit/pull/515)
