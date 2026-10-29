<p align="center">
  <a href="https://ifiokjr.github.io/leptoskit/"><img src="https://raw.githubusercontent.com/ifiokjr/leptoskit/main/docs/src/assets/mark.svg" alt="leptoskit" width="64"></a>
</p>

# leptoskit_plugin_router

> The file router as a leptoskit plugin: generates the typed `Route` module before cargo runs and answers `leptoskit plugin:router routes`.

Routes are files under `src/routes/`, and the app needs a typed `Route` enum with a variant for each. Something has to write that enum before cargo compiles the app. That is this crate: a build-tool plugin that runs the generator in `leptoskit_router_codegen` when `leptoskit dev`, `leptoskit build`, or `leptoskit native` starts, and writes `src/routes.rs` only when the routes changed.

It is the tooling half of file routing. Apps depend on [`leptoskit_router`](https://github.com/ifiokjr/leptoskit/tree/main/crates/leptoskit_router) for the runtime (`Route`, `Routable`, `Router`), which this crate re-exports as `leptoskit_plugin_router::router`, with the generator as `leptoskit_plugin_router::codegen`, so tooling has one import point. You do not add this crate to an app's dependencies; the `leptoskit` CLI links it.

Turn it on in `leptoskit.toml`:

```toml
[[plugins]]
name = "router"

[plugins.options]
# Optional; the defaults suit src/routes/ -> src/routes.rs.
dir = "src/routes"
output = "src/routes.rs"
```

Run the command by hand after adding a screen, or in CI to catch a forgotten regeneration:

```sh
leptoskit plugin:router routes            # write the module and list every route, most specific first
leptoskit plugin:router routes --check    # fail instead of writing when the committed module is stale
```

`leptoskit routes` still works as an alias that forwards here. Formatting does not count as a change, so rustfmt can format the generated file, and leptoskit never overwrites a `routes.rs` it did not write. The directory and output paths are validated to stay inside the project.

[`examples/file_routes`](https://github.com/ifiokjr/leptoskit/tree/main/examples/file_routes) is a complete app, and the book's file routes guide covers the naming conventions (`[note].rs`, layouts, `+not-found.rs`).
