# file_routes

Field, a notes app whose screens are files. Every file under `src/routes/` is a route, and `src/routes.rs` is generated from them: a module per file and the `Route` enum.

The example is a standalone workspace in the shape `leptoskit new` produces — one shared application, and runner projects for iOS, Android, macOS, Windows, and Linux — so every destination builds through the standard commands:

```bash
cd examples/file_routes
leptoskit dev                        # the web app, rebuilding on change
leptoskit build --platform ios       # or android, macos, windows, linux
leptoskit routes                     # regenerate src/routes.rs and list the routes
leptoskit routes --check             # fail when src/routes.rs is out of date
```

`tests/guest.rs` loads the built native guest into the interpreter and presses its links; run `leptoskit native bundle --target ios` in this directory first to produce the bundle it reads.

The other tests parse every URL the files define and write each route's href back; render every route inside its layouts on the headless backend; tap through the app's links and back button; and, in a browser (`test:browser`), check that links add history entries the browser's back button returns through.
