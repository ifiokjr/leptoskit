<h1 align="center">leptoskit_router</h1>

<p align="center">
  <a href="https://ifiokjr.github.io/leptoskit/"><img src="https://raw.githubusercontent.com/ifiokjr/leptoskit/main/docs/src/assets/mark.svg" alt="leptoskit" width="64"></a>
</p>

<p align="center">
  <strong>leptoskit_router</strong> is the runtime behind leptoskit file routes: `src/routes/` becomes one `Route` enum that parses URLs and writes hrefs.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://docs.rs/leptoskit_router"><strong>Docs</strong></a> ·
  <a href="https://github.com/ifiokjr/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<p align="center">
  <a href="https://crates.io/crates/leptoskit_router"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_router.svg"></a>
  <a href="https://docs.rs/leptoskit_router"><img alt="docs.rs" src="https://docs.rs/leptoskit_router/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<br>

## Overview

Routes are files. `leptoskit routes` reads `src/routes/` and writes `src/routes.rs`: a module for every route file, and one `Route` enum with a variant per route. This crate is the runtime that enum stands on:

- `Routable` — what the generated `Route` implements: `Route::parse("/notes/42")` and `route.href()`;
- `#[derive(Route)]` — a route struct whose fields are the route's parameters, read from the path when a field is named after a `[segment]` and from the query string otherwise;
- `RouteUrl`, `Href`, `Pattern` — URL decoding that reads any URL the way a browser does, href encoding that never writes an empty or dot segment, and matching with optional segments and catch-alls;
- `NotFound` — the unmatched path, for a `+not-found.rs` route;
- `Router` and `Render` — the app's current route and its history, rendered as the route's page inside its layouts, with links that render as `<a href>` on the web. The `web` feature keeps a browser router in step with the address bar and the back button.

The [file routes guide](https://ifiokjr.github.io/leptoskit/guide/10-file-routes.html) covers the naming conventions, and [`examples/file_routes`](https://github.com/ifiokjr/leptoskit/tree/main/examples/file_routes) is a complete app.

## How it fits

The CLI's `leptoskit routes` runs the codegen; `leptoskit_plugin_router` runs it before cargo; this crate is what app code imports. A route file is the only place a URL shape is declared, so links and parsing cannot drift.

## Use it

```rust
use leptoskit_router::Route;

/// `src/routes/notes/[note].rs`, matching `/notes/:note`.
#[derive(Route, Clone, Debug, PartialEq)]
pub struct Note {
	pub note: u64,              // the [note] segment
	pub highlight: Option<u32>, // ?highlight=
}
```

## Learn more

- [Crate documentation](https://docs.rs/leptoskit_router)
- [leptoskit documentation](https://ifiokjr.github.io/leptoskit/)
- [Repository](https://github.com/ifiokjr/leptoskit)
- [File routes](https://ifiokjr.github.io/leptoskit/guide/10-file-routes.html)
