<p align="center">
  <a href="https://ifiokjr.github.io/leptoskit/"><img src="https://raw.githubusercontent.com/ifiokjr/leptoskit/main/docs/src/assets/mark.svg" alt="leptoskit" width="64"></a>
</p>

# leptoskit_router

> Typed file routes for leptoskit.

Routes are files. `leptoskit routes` reads `src/routes/` and writes `src/routes.rs`: a module for every route file, and one `Route` enum with a variant per route. This crate is the runtime that enum stands on:

- `Routable` — what the generated `Route` implements: `Route::parse("/notes/42")` and `route.href()`;
- `#[derive(Route)]` — a route struct whose fields are the route's parameters, read from the path when a field is named after a `[segment]` and from the query string otherwise;
- `RouteUrl`, `Href`, `Pattern` — URL decoding, href encoding, and matching with optional segments and catch-alls;
- `NotFound` — the unmatched path, for a `+not-found.rs` route.

```rust
use leptoskit_router::Route;

/// `src/routes/notes/[note].rs`, matching `/notes/:note`.
#[derive(Route, Clone, Debug, PartialEq)]
pub struct Note {
	pub note: u64,              // the [note] segment
	pub highlight: Option<u32>, // ?highlight=
}
```

The [file routes guide](https://ifiokjr.github.io/leptoskit/guide/10-file-routes.html) covers the naming conventions, and [`examples/file_routes`](https://github.com/ifiokjr/leptoskit/tree/main/examples/file_routes) is a complete app.
