<h1 align="center">leptoskit_plugin_router</h1>

<p align="center">
  <a href="https://ifiokjr.github.io/leptoskit/"><img src="https://raw.githubusercontent.com/ifiokjr/leptoskit/main/docs/src/assets/mark.svg" alt="leptoskit" width="64"></a>
</p>

<p align="center">
  <strong>leptoskit_plugin_router</strong> is the file router as a leptoskit plugin: it generates the typed `Route` module before cargo runs and answers `plugin:router routes`.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://docs.rs/leptoskit_plugin_router"><strong>Docs</strong></a> ·
  <a href="https://github.com/ifiokjr/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<p align="center">
  <a href="https://crates.io/crates/leptoskit_plugin_router"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_plugin_router.svg"></a>
  <a href="https://docs.rs/leptoskit_plugin_router"><img alt="docs.rs" src="https://docs.rs/leptoskit_plugin_router/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<br>

## Overview

Routes are files under `src/routes/`, and the app needs a typed `Route` enum with a variant for each. Something has to write that enum before cargo compiles the app. That is this crate: a build-tool plugin that runs the generator in `leptoskit_router_codegen` when `leptoskit dev`, `leptoskit build`, or `leptoskit native` starts, and writes `src/routes.rs` only when the routes changed.

It is the tooling half of file routing. Apps depend on [`leptoskit_router`](https://github.com/ifiokjr/leptoskit/tree/main/crates/leptoskit_router) for the runtime (`Route`, `Routable`, `Router`), which this crate re-exports as `leptoskit_plugin_router::router`, with the generator as `leptoskit_plugin_router::codegen`, so tooling has one import point. You do not add this crate to an app's dependencies; the `leptoskit` CLI links it.

Turn it on in `leptoskit.toml`.

Run the command by hand after adding a screen, or in CI to catch a forgotten regeneration:

```sh
leptoskit plugin:router routes            # write the module and list every route, most specific first
leptoskit plugin:router routes --check    # fail instead of writing when the committed module is stale
```

`leptoskit routes` still works as an alias that forwards here. Formatting does not count as a change, so rustfmt can format the generated file, and leptoskit never overwrites a `routes.rs` it did not write. The directory and output paths are validated to stay inside the project.

[`examples/file_routes`](https://github.com/ifiokjr/leptoskit/tree/main/examples/file_routes) is a complete app, and the book's file routes guide covers the naming conventions (`[note].rs`, layouts, `+not-found.rs`).

## How it fits

Generated apps enable it by default, so `src/routes/` is always in step with the build. The runtime and derive live in `leptoskit_router`; this crate only owns the build hook.

## Use it

```toml
[[plugins]]
name = "router"

[plugins.options]
# Optional; the defaults suit src/routes/ -> src/routes.rs.
dir = "src/routes"
output = "src/routes.rs"
```

## Learn more

- [Crate documentation](https://docs.rs/leptoskit_plugin_router)
- [leptoskit documentation](https://ifiokjr.github.io/leptoskit/)
- [Repository](https://github.com/ifiokjr/leptoskit)
- [File routes](https://ifiokjr.github.io/leptoskit/guide/10-file-routes.html)
