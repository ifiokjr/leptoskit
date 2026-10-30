# Commonplace

Commonplace is a research notebook, named after the commonplace books people kept of passages worth remembering. You keep research projects, such as "Urban heat islands", clip quotes into them from what you read, write notes, and export a bibliography in APA, MLA, or BibTeX.

It is one product on every surface leptoskit builds, and the worked example for the [extension platform](../../docs/agents/extension-plan.md):

| Surface                                                | What it is                                                                                                 |
| ------------------------------------------------------ | ---------------------------------------------------------------------------------------------------------- |
| Server (`src/main.rs`)                                 | The JSON API, sign-in with emailed codes, and the built website, on SQLite.                                |
| Website                                                | The library in the browser, rendered client-side.                                                          |
| Native apps (`ios/`, `android/`, `macos/`)             | The same screens as native widgets.                                                                        |
| Browser extension (`commonplace_app/src/extension.rs`) | Clipping from any page, a side panel, highlights, and an offline queue, for Chromium, Firefox, and Safari. |

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

## The extension

The extension is the same app at a few more routes, configured in `[extension]` in `leptoskit.toml`:

| Surface                     | Route                 | What it does                                                                                                      |
| --------------------------- | --------------------- | ----------------------------------------------------------------------------------------------------------------- |
| Popup                       | `/clip`               | Clip the page you're on, or open the clipper on the page                                                          |
| Side panel                  | `/panel`              | A project's clips beside the page; pressing one opens its page                                                    |
| Options                     | `/settings`           | Sign in, the project clips go to, and the sites that show highlights                                              |
| New tab                     | `/reading`            | Your projects, on Chromium and Firefox                                                                            |
| `clipper` content script    | `/overlay/clipper`    | A frame beside the selection, opened by the toolbar, the context menu, or `Alt+Shift+C`; needs no host permission |
| `highlights` content script | `/overlay/highlights` | Highlights your clips on the sites you allowed, and shows one when you press it                                   |
| Background worker           |                       | The context menu, the shortcut, the badge, and the queue of clips saved while the server was unreachable          |

The extension signs in for a bearer token and keeps it in extension storage. It reaches the server named in `[extension].server`.

```bash
leptoskit build --platform extension
```

Then load `.leptoskit/extension/chromium` unpacked at `chrome://extensions`, with developer mode on, or `.leptoskit/extension/firefox/manifest.json` from `about:debugging` in Firefox.

## Test it

```bash
cargo test -p commonplace_shared -p commonplace_backend -p commonplace_app
leptoskit db check
```

- `commonplace_backend/tests/api.rs` signs in with an emailed code and drives every route, including that one account can't see another's research.
- `commonplace_app/tests/journey.rs` drives the real screens on the headless backend against the server in-process: sign in, start a project, clip a quote, write a note, and export the bibliography.

After changing a table, run `leptoskit db migration create <name>` and commit what it writes.
