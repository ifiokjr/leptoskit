# Commonplace

Commonplace is a research notebook, named after the commonplace books people kept of passages worth remembering. You keep research projects, such as "Urban heat islands", clip quotes into them from what you read, write notes, and export a bibliography in APA, MLA, or BibTeX.

It is one product on every surface leptoskit builds, and the worked example for the [extension platform](../../docs/agents/extension-plan.md):

| Surface                                    | What it is                                                                  |
| ------------------------------------------ | --------------------------------------------------------------------------- |
| Server (`src/main.rs`)                     | The JSON API, sign-in with emailed codes, and the built website, on SQLite. |
| Website                                    | The library in the browser, rendered client-side.                           |
| Native apps (`ios/`, `android/`, `macos/`) | The same screens as native widgets.                                         |
| Browser extension                          | Clipping from any page, added as the extension platform lands.              |

## Where things live

| Path                  | What it holds                                                                                                                       |
| --------------------- | ----------------------------------------------------------------------------------------------------------------------------------- |
| `commonplace_db`      | The tables: projects, sources, clips, and notes. Each belongs to an account, and each table's policy shows it only to that account. |
| `commonplace_shared`  | The API's request and response types, tags, and the citation styles, which every surface formats the same way.                      |
| `commonplace_backend` | The API's routes.                                                                                                                   |
| `commonplace_app`     | The screens, as file routes in `src/routes/`; the store they read; and the API client with one transport per host.                  |
| `migrations/`         | The schema's history, written by `leptoskit db migration create`.                                                                   |

Every client reaches the server through the same API:

- **The website** sends the session cookie that sign-in sets, which scripts can't read.
- **The native apps** sign in for a bearer token, send it through the HTTP plugin, and keep it in the secure store between launches.

## Run it

```bash
cd examples/commonplace
leptoskit build                 # the website, into dist/
cargo run                       # the server, on http://127.0.0.1:3000
```

Without `AUTH_ENCRYPTION_KEYS`, a debug server uses a development key and keeps every email at `/_leptoskit/mail`, so you can read your sign-in code there.

To run the macOS app against that server:

```bash
leptoskit native build macos
open .leptoskit/macos/App.app
```

The native apps reach `http://127.0.0.1:3000` by default. To use another server, set `COMMONPLACE_SERVER` when you build, and add its origin to `[native.plugin-options.http]` in `leptoskit.toml`. An Android emulator reaches this machine's server after `adb reverse tcp:3000 tcp:3000`.

## Test it

```bash
cargo test -p commonplace_shared -p commonplace_backend -p commonplace_app
leptoskit db check
```

- `commonplace_backend/tests/api.rs` signs in with an emailed code and drives every route, including that one account can't see another's research.
- `commonplace_app/tests/journey.rs` drives the real screens on the headless backend against the server in-process: sign in, start a project, clip a quote, write a note, and export the bibliography.

After changing a table, run `leptoskit db migration create <name>` and commit what it writes.
