# file_routes

Field, a notes app whose screens are files. Every file under `src/routes/` is a route, and `src/routes.rs` is generated from them: a module per file and the `Route` enum.

```bash
cd examples/file_routes
leptoskit routes          # regenerate src/routes.rs and list the routes
leptoskit routes --check  # fail when src/routes.rs is out of date
cargo test -p file_routes_example
```

The tests parse every URL the files define and write each route's href back; render every route inside its layouts on the headless backend; tap through the app's links and back button; and, in a browser (`test:browser`), check that links add history entries the browser's back button returns through.
