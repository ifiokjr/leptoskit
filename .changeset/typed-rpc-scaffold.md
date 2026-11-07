---
"release": minor
---

# New apps publish a typed RPC

A new app's API is now typed operations that every platform calls by name and every generated client describes. `leptoskit rpc spec` and `leptoskit rpc generate` work on a fresh scaffold: the server writes the published API and exits before it reads its configuration, opens the database, or loads sign-in's keys, so exporting needs no `DATABASE_URL` and no secrets, and creates no database file.

- Every new app publishes its backend's `health` check, still `GET /api/health`, as an `#[api]` server function, and `<name>_shared`'s `Health` derives `JsonSchema`.
- With `--db` or `--auth`, `<name>_shared` declares a `notes` query and an `add_note` mutation. The browser, the native apps, and the server call them by their argument structs, `Notes {}` and `AddNote { body }`; only the shared crate's `server` feature keeps their bodies, so no client links the database driver. The server serves them through `leptoskit::rpc::server::Server` and over the live socket at `/live`, with a `live` readiness check, and on `SIGTERM` it tells live sockets to reconnect before it drains. The `Note` table is `live`, and the db crate adds the `rpc` schema pack, whose receipts make a re-sent mutation run once.
- With `--auth`, operations run as whoever signed in: each note has an `owner_id`, and the table's policy shows and changes only the caller's own. The live socket follows `[auth] origin` once it's set. `/healthz` and `/readyz` sit outside the sign-in layer, `/readyz` waits for the migrations, and the server drains on shutdown like the others.

The scaffold's notes API changed shape, in new apps only: `GET /api/notes` is now `POST /api/notes` with `{}`, and `POST /api/notes` with `{"body": …}` is now `POST /api/add_note`. A failed call answers `Kind|message` text, and an empty note is `400` rather than `422`; `404`, `409`, and `500` keep their meaning, and with sign-in a signed-out call answers `401`. Apps created by an earlier leptoskit keep the handlers they have.

In a native app's guest, which has no timer, the live client no longer connects when it's configured: it waits until there's a call, a subscription, or a channel to serve, or calls in the offline queue. Before, an app that configured the client without going live, as sign-in's `configure` does once the app has operations, retried a socket it couldn't open without pause and never finished starting. `leptoskit_query::has_timer` says whether a host's `sleep` really waits.
