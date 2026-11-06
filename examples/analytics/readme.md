# Analytics: a service beside the app

Product analytics usually shouldn't run inside the server that renders your pages. A burst of events, or a slow report, would slow every page down, and the analytics data needs a database of its own. This example puts analytics in a **service**: a separate program with its own database, which the app starts, calls, and watches as if its operations were the app's own.

It targets the **web**: a server-rendered page that hydrates in the browser.

## What's in it

- **`analytics_service`** is the service, and a service plugin other apps could add.
  - Its library defines an `events` table and three operations, each marked `service = "analytics"`:
    - `capture` records an event, and anyone may call it;
    - `summary` counts events per name and per day, and anyone may watch it live;
    - `recent` lists individual events, and only the app may call it.
  - Its binary opens and migrates its own database, then serves those operations under `/_leptoskit/services/analytics/`.
  - Its `Cargo.toml` tells leptoskit how to run it, in `[package.metadata.leptoskit.service]`: its port, that browsers may reach it, and that it needs a database.
- **`analytics_app`** is a leptoskit app with one line of setup:

  ```toml
  [services.analytics]
  package = "analytics_service"
  ```

  Its page has two buttons that send events from the browser, and a summary that the server renders first and that then updates live as events arrive.

The app depends on the service's library for its types, so calling an operation is the same code everywhere:

```rust
call(Capture { event: "clicked_buy".into(), distinct_id, properties }).await
```

Where that call goes depends on where it runs:

- **In the browser**, it posts to `/_leptoskit/services/analytics/api/capture` on the app's own address, and leptoskit forwards it to the service.
- **On the app's server**, while it renders the page, it goes straight to the service, with the service's secret token. That token is what lets the app read `recent`.

The app's server never runs the operations itself: the bodies are compiled only into the service.

## Run it

```sh
cd examples/analytics/analytics_app
leptoskit dev
```

`leptoskit dev` builds the app and the service together. It then starts the service with a SQLite database in `.leptoskit/services/analytics/`, waits until the service is ready, and starts the app. Both answer at <http://127.0.0.1:3000>: pages from the app, and `/_leptoskit/services/analytics/` from the service. `leptoskit services` lists what it will start.

`leptoskit build` followed by `leptoskit serve` runs the same thing from a build.

If the app's own database is Postgres, the service gets a database of its own on the same Postgres server instead. Add `features = ["service", "postgres"]` to the app's `[services.analytics]` table, so the service is built with Postgres support.

## Changing the table

The service applies its migrations itself when it starts. After changing `Event`, regenerate them. In `analytics_service`, write a temporary `leptoskit.toml` that points `[db]` at the crate, then create the migration:

```toml
schema_version = 0
[app]
name = "analytics_service"
[db]
dialect = "sqlite"
crate = "analytics_service"
features = ["service"]
migrations = "migrations"
```

```sh
leptoskit db migration create <name>
```

Repeat with `dialect = "postgres"` and `migrations = "migrations-postgres"`, then delete the temporary file.

## Tests

- `cargo test -p analytics_service --features service` runs the service on a real port. It checks who may call what, and that a live summary updates when an event arrives.
- `cargo test -p analytics_app --features ssr` builds the app with the service's code linked in, as `cargo test --workspace` and `leptoskit build` do. It checks that the app doesn't serve the service's operations itself, and that calling one reaches the service.
