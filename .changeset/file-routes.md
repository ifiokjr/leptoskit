---
"release": minor
---

# Route apps by their files

Put one file per screen in `src/routes/`, add a `[router]` table to `leptoskit.toml`, and leptoskit writes `src/routes.rs`: a module for every route file and one `Route` enum that parses URLs and writes hrefs. The file names follow Expo Router: `index.rs`, `[note].rs`, `[[lang]]/`, `[...topic].rs`, `[[...topic]].rs`, `(group)/`, `_layout.rs`, `_private.rs`, `+not-found.rs` and `+html.rs`. A route with parameters declares a `#[derive(Route)]` struct whose fields come from the path when they are named after a segment and from the query string otherwise, so a link to a missing route, or one without its parameters, does not compile. Pages are a lowercase `page` function or a capitalised `#[component] Page`.

`leptoskit routes` writes the module and lists the routes, and `--check` fails in CI when the committed module is out of date. `dev`, `build` and `native` regenerate it before compiling, writing only when the routes changed. The generator reports every problem with its file and line, and a route struct that drifts from its file name is a compile error even before the generator runs again. The new crates are `leptoskit_router`, `leptoskit_router_macros` and `leptoskit_router_codegen`; `examples/file_routes` is a complete app.
