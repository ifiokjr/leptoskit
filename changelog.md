# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.1](https://github.com/ifiokjr/leptoskit/releases/tag/v0.1.1) (2026-10-30)

Grouped release for `release`.

### Features

- **Add the leptoskit platform and the release pipeline.** Publish placeholder: exercises the changeset flow, the release pull request, and crates.io publishing end to end before real changes ship through them. This release carries the whole platform — the yoga layout engine, the native renderer, the wasm runtime host, code push, the Axum server toolkit, the server-fn API, configuration, plugins, and the CLI — plus the release machinery itself. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #3](https://github.com/ifiokjr/leptoskit/pull/3)

#### Add `leptoskit_query`, TanStack Query for leptos

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #110](https://github.com/ifiokjr/leptoskit/pull/110)

A new crate for async state management built on leptos's fine-grained signals: cached, deduplicated, retrying queries with stale-while-revalidate semantics, infinite queries that page in either direction, and mutations with lifecycle callbacks and optimistic updates. Queries refetch on window focus, reconnect, and interval; pause while offline; garbage collect when unobserved; and can stream server-fetched data to the browser through the same hydration mechanism leptos resources use.

Caches sync across browser tabs over a `BroadcastChannel`, or across the windows of a native app over any `BroadcastTransport`. Browser integration sits behind a default `browser` feature, so a wasm bundle hosted outside a browser — an iOS or Android shell — builds without it and installs its own clock and timer. The `leptoskit` meta crate re-exports it behind the `query` feature.

#### Harden code push and layout; add the logo and docs

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #14](https://github.com/ifiokjr/leptoskit/pull/14)

Code push now rejects bundle paths that escape the staging directory, bounds decompression and delta output before allocating, refuses to overwrite a published release, writes releases atomically, offers deltas only against the exact release they were computed from, and falls back to the full download when a delta fails.

The layout engine caches measurements per node, so layout cost no longer grows exponentially with nesting depth. Flex free space, percentage bases, reversed axes, single-line `align-content`, grid placement of partly-auto items, grid row sizing, `minmax` growth, and `display: contents` in grid now follow CSS. Tree cycles and huge grid line numbers are rejected.

The native renderer inserts a replaced element once, relayouts when text changes, and forgets deleted nodes. The wasm host clamps guest indices, bounds-checks guest pointers, rejects tree cycles, derives the root at commit, and reports `OutOfFuel`.

The CLI always lets cargo decide what to recompile and caches only wasm-bindgen and `wasm-opt`, honours `build.client_package`, features, plugin env, and rustflags, restores missing outputs, and scaffolds projects that build. `doctor` exits non-zero when a required tool is missing. `leptoskit_axum` sends `Vary: Accept-Encoding`, limits the SPA fallback to page routes, and errors on an invalid bind address. `leptoskit_rpc` retries work in the browser. `leptoskit_config` validates package ids and code push settings.

The project has a logo, a documentation site, and a brand page.

#### Ship prebuilt leptoskit binaries on the GitHub release

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #18](https://github.com/ifiokjr/leptoskit/pull/18)

The publish workflow now builds the `leptoskit` CLI for eight targets — linux (gnu and musl) on both architectures, macOS on both architectures, and windows-msvc on both architectures — and attaches the archives and their sha256 and sha512 checksums to the release, with build-provenance attestations verified before anything is published. The release only becomes public after the binaries are attached and every crate is on crates.io, so a published version always has installable artifacts.

The CLI crate carries `cargo-binstall` metadata resolving to those archives, so `cargo binstall leptoskit_cli` installs the prebuilt binary instead of compiling from source.

#### Add the AppKit backend and styled native elements

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #22](https://github.com/ifiokjr/leptoskit/pull/22)

leptoskit renders on a real platform: `leptoskit_appkit` commits mutation batches as AppKit views on macOS, measures text with AppKit's own font metrics, delivers button and text-field events, opens windows, and renders view trees to PNG offscreen.

Elements gain cross-platform styling: corner radius, borders, opacity, font size and weight, and text alignment, with old wire JSON still decoding. `TextMeasurer::measure` now receives the text style, so measurement follows the font.

The layout engine measures column items at the column's width so wrapped text gets its real height, caps content by `max_width`, sizes the cross axis of ratio-bound items from `aspect_ratio`, and resolves percentages inside stretched columns.

A new `gallery` example renders six screens through the AppKit backend, published on the documentation site's examples page.

- **Add native OS UI automation through XCUITest, UI Automator 2, Windows UI Automation, and Linux AT-SPI, with real PNG screenshots, visual failure reports, and separate colored layout diagnostics.** _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #39](https://github.com/ifiokjr/leptoskit/pull/39)
- **Publish the RPC: generated clients for any language.** `#[leptoskit_rpc::api]` above `#[server]` publishes a server function: the route is pinned to a stable `/api/<name>`, the doc comment (including its `# Arguments` section) documents the function and each argument, and JSON Schemas generated through schemars describe every payload. `#[api(internal)]` leaves a function callable but takes it out of generated clients. The server exports the description on request (`leptoskit_rpc::spec::export_if_requested`), and the new `leptoskit rpc` CLI commands consume it: `rpc list` and `rpc spec` (leptoskit JSON, OpenAPI 3.1 JSON or YAML), and `rpc generate`, which writes typed clients that speak the RPC's own protocol — TypeScript over fetch, Python over the standard library with an optional httpx async client, Go over net/http, Rust over reqwest, Dart over package:http, and a clap command line with one command per function — plus the OpenAPI document. Scaffold modes decide what regeneration may rewrite, and a new `[rpc]` section in leptoskit.toml names the clients, their directories, and exclusion globs. Two new crates carry the machinery: `leptoskit_rpc_macros` and `leptoskit_rpc_codegen`. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #119](https://github.com/ifiokjr/leptoskit/pull/119)

#### Generate one application for web and native platforms

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #148](https://github.com/ifiokjr/leptoskit/pull/148)

`leptoskit new` creates a workspace with a shared application, backend, and shared domain crate, plus web, iOS, Android, macOS, Windows, and Linux runners. All six platforms are enabled by default. Builds select a platform feature and isolate its generated state and application bundle.

Builds select registered `.platform.rs` module overrides and compile the shared application into a browser bundle or native Wasm guest. App-owned platform runners keep native dependencies separate from the application. Native code-push releases use a channel for each platform so incompatible runtime shells cannot block one another's updates.

#### Share application components across platform builds

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #147](https://github.com/ifiokjr/leptoskit/pull/147)

Add a native Wasm guest adapter for portable component trees and explicit iOS, Android, macOS, Windows, and Linux feature markers. Native SDK dependencies follow the actual compilation target, so destination features remain usable by Wasm applications.

The new `leptoskit_build` helper and `platform_module!` macro select platform source overrides with a shared fallback, track file additions and removals, and reject conflicting destination features.

- **Rename leptoskit_fns to leptoskit_rpc.** The crate's job is publishing the RPC — describing it and generating clients — so the name now says so: `leptoskit_fns` becomes `leptoskit_rpc`, with `leptoskit_rpc_codegen` and `leptoskit_rpc_macros` alongside. The `leptoskit` meta crate's `fns` feature and module become `rpc`, the CLI command becomes `leptoskit rpc` (spec/list/generate unchanged), the configuration section becomes `[rpc]`, and the export environment variables become `LEPTOSKIT_RPC_*`. The retry/query layer the crate started as keeps its API under the new name. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #152](https://github.com/ifiokjr/leptoskit/pull/152)

#### file_routes everywhere; apps draw pressables

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #193](https://github.com/ifiokjr/leptoskit/pull/193)

`examples/file_routes` is now a standalone workspace in the shape `leptoskit new` produces, so `leptoskit build --platform ios` — and every other destination — builds and runs it; CI builds it for each platform, launches it on an iOS simulator and an Android emulator, and drives its router through the interpreter.

Generated macOS, iOS, and Android apps now draw `Pressable` in addition to the six components they already supported, so file-router links work in generated apps.

The native Wasm runtime also resolves wasm-bindgen's linked-but-unused runtime imports, so guests whose dependency graphs pull in wasm-bindgen without calling a browser API — like `reactive_graph`, which the file router links — instantiate instead of failing with an unresolved-import error. Real browser imports still fail loudly.

- **Async storage adapters for store and secure-store plugins.** `leptoskit_plugin_store` and `leptoskit_plugin_secure_store` now implement `leptoskit_use::AsyncStorage` behind an `async-storage` feature, so `use_async_storage` persists shared state through the store plugin's settings document on native and browser builds, and through the OS credential store for small secrets on desktop and native guests. Adapter writes commit before signals update, failures preserve the last committed value, and storage keys that cannot serve as credential names are stored under SHA-256 digests. The browser suite exercises the adapter against real local storage, including rejected writes. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #197](https://github.com/ifiokjr/leptoskit/pull/197)
- **Signal hooks for battery, network, location, and clipboard.** `leptoskit_plugin_battery`, `leptoskit_plugin_network`, `leptoskit_plugin_location`, and `leptoskit_plugin_clipboard` gain a `hooks` feature with leptos-use-style signal hooks: `use_battery`, `use_network`, `use_geolocation`, and `use_clipboard`. Each hook returns a signal plus one driving future handed to the host executor — nothing is spawned internally. Snapshot hooks publish the first reading, then follow platform changes until the reactive owner is disposed or a platform call fails, leaving the last value in place; the clipboard hook mirrors successful read, write, and clear actions in a signal. Each crate exposes a sealed `Source` trait implemented by the interpreter client and a browser source, with in-crate test doubles covering the loops natively. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #224](https://github.com/ifiokjr/leptoskit/pull/224)
- **Portable utilities for leptoskit_use.** `leptoskit_use` mirrors more of leptos-use's portable surface: `use_timestamp` reports elapsed host-clock time since creation, `watch_debounced` and `watch_throttled` add trailing-edge debounced and leading-edge throttled watchers on the shared clock, `use_async_state` loads a signal from a caller-driven async operation with explicit pending/success/error state, and `use_supported` wraps capability probes as signals. Like the rest of the crate, none of them spawn work: the host drives every future and timer. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #200](https://github.com/ifiokjr/leptoskit/pull/200)
- **Browser-only hooks behind the leptoskit_use web feature.** `leptoskit_use` gains an optional `web` feature that compiles a `leptoskit_use::web` module on browser targets only: `use_event_listener` for DOM events, `use_media_query` for CSS media queries, and the `use_preferred_dark` and `use_preferred_reduced_motion` settings hooks. Listeners are scoped to the reactive owner and also stop through a returned handle; without a window, or for rejected queries, media signals stay `false`. Enabling `leptoskit`'s `web` feature turns the module on wherever `leptoskit::use` is enabled, so web builds pick it up automatically while native guests never compile it. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #200](https://github.com/ifiokjr/leptoskit/pull/200)

#### Identify interpreter and transport owners

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #159](https://github.com/ifiokjr/leptoskit/pull/159)

Bridge and WasmHost expose opaque cloneable identity handles that remain stable across moves and cannot collide while an earlier handle remains alive. Services can bind work to both owners and reject delivery after replacement or transport transplantation. Handles retain no guest or interpreter state.

App::with_host lends the application's actual interpreter to a synchronous Rust closure. Native I/O can retain owned work while the application remains usable; interpreter references and futures borrowing them cannot escape the closure.

#### The file router is a plugin with its own commands

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #195](https://github.com/ifiokjr/leptoskit/pull/195)

Routing is now declared like every other plugin — `[[plugins]] name = "router"` in `leptoskit.toml` — with `dir` and the new `output` options placing the generated module. The router generates through the same plugin lifecycle as everything else, so `leptoskit build`, `dev`, and `native` regenerate the module through the plugin registry, and the dev watcher excludes whatever any plugin generates.

Its commands live under the new plugin namespace: `leptoskit plugin:router routes` (and `--check`) replace the top-level `leptoskit routes`, which still forwards with a deprecation note. `leptoskit --help` lists the plugin commands a project activates. `leptoskit migrate` rewrites a legacy `[router]` table into the plugin form; the table keeps working until then.

#### Plugins can be separate programs anyone can write

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #196](https://github.com/ifiokjr/leptoskit/pull/196)

A `[[plugins]]` entry with a `command` — or an executable named `leptoskit-plugin-<name>` on PATH — is now a program leptoskit runs once per hook, with a versioned JSON envelope on standard input and a JSON reply on standard output. Any language works, nothing is linked into the CLI, and a failed program fails the hook loudly by name.

The program answers `describe` with its CLI namespace, supported platforms, and commands; `generate` with the files it wrote; the build hooks with extra cargo features, environment, and rustflags; and its own `plugin:<name> <command>` invocations with an exit code. `leptoskit generate tool-plugin <dir>` scaffolds a working Rust program implementing the protocol, ready for `cargo install --path`.

- **Register application-owned asynchronous native services.** Native runtime metadata accepts `rust-service-factory` for an inert `ServiceInstaller`. Generated macOS, iOS and Android Rust registrants defer the service until platform activation owns wake notifications and joined cleanup. Ambiguous plugin/service constructors and multiple selected service registrations fail before generation. Existing plugin and worker registration retain their behavior. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #169](https://github.com/ifiokjr/leptoskit/pull/169)

#### One dependency for the server

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #192](https://github.com/ifiokjr/leptoskit/pull/192)

A server package now depends on `leptoskit` alone. The new `server` feature re-exports the server toolkit under `leptoskit::server` together with the crates every deployment links — `axum`, `tokio`, `tower`, `tower-http`, and `leptos_axum` — at the versions leptoskit was built against; `tokio` carries the `macros` and `rt-multi-thread` features, so `use leptoskit::server::tokio;` is all `#[tokio::main]` needs, and `tower` and `tower-http` carry the limit, load-shed, and timeout layers. `leptoskit_axum` re-exports the same four crates and gains `serve_router` and `serve_listener`, which serve an app's own router until `SIGINT` or `SIGTERM` and finish the requests in flight first, the way `serve` always has.

`leptos`, `leptos_router`, and `leptos_meta` are re-exported behind the `leptos` feature, and the new `csr`, `hydrate`, `ssr`, and `islands` features forward to them, so a package can write `hydrate = ["leptoskit/hydrate"]` instead of naming leptos; `server` implies `ssr`. `leptoskit build` turns islands on through `leptoskit/islands` when a package reaches leptos that way, and only requires a direct dependency on one of the two. A crate that uses leptos's macros still depends on `leptos` itself, because they expand to `::leptos::` paths.

`leptoskit_codepush` re-exports `ed25519_dalek` and `semver`, whose types sign and number every release.

`leptoskit new` generates a server and a backend that depend on `leptoskit` only, and the server now serves `pkg/` and `assets/` with their precompressed copies and stops cleanly on `SIGTERM`.

#### Add `leptoskit_l10n`: typed translations from ARB files

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #191](https://github.com/ifiokjr/leptoskit/pull/191)

Messages live in one ARB file per locale (`locales/en.arb`, `locales/pt-BR.arb`), with placeholders, ICU plurals (`{count, plural, =0 {none} one {# item} other {# items}}`), selects, and `@key` metadata that documents a message and types its placeholders. Turn translations on with an `[l10n]` table in `leptoskit.toml`, and the new `leptoskit l10n` command writes `src/l10n.rs`: a `Messages` trait with one method per message, typed by its placeholders and documented with its text; one implementation per locale, with untranslated messages falling back to the template; and the `L10n` handle an app reads them through. `dev`, `build`, and native builds regenerate the module before compiling, and `leptoskit l10n --check` fails CI when it is stale. Every problem in the files is reported with its file and line.

The runtime crate, `leptoskit_l10n`, carries `Locale` tags, Flutter-style locale negotiation, CLDR plural rules for every language, an `Accept-Language` parser for server rendering, `navigator.languages` for the browser behind its `web` feature, and the reactive `L10n` handle: a view that reads a message renders again when the locale changes. `leptoskit_l10n_codegen` is the generator the CLI uses. The `leptoskit` meta crate re-exports the runtime behind the `l10n` feature.

#### `leptoskit dev` runs every platform

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #213](https://github.com/ifiokjr/leptoskit/pull/213)

`leptoskit dev` now runs every platform in `app.platforms` that this machine can run, and keeps them all current: the web app as before, plus each native app on this machine, in every booted iOS Simulator, and on every attached Android device or emulator. When no simulator or emulator is running, the newest iPhone simulator or the first installed emulator boots. Enabled platforms this machine can't run are skipped with the reason. `--platform`, repeatable, runs only the platforms named and fails when one can't run here. App logs stream into the same terminal, each line marked with its platform. `--device`, also repeatable, narrows iOS and Android to the simulators (by name or UDID) and adb serials named, booting a named simulator that is shut down. Devices that never finish booting or stop answering adb are skipped with the reason instead of stalling `dev`.

Each native app is built and installed once. After that, a change rebuilds only its WebAssembly interface, which `dev` serves from a loopback server (Android reaches it through `adb reverse`); the running app swaps it in without being reinstalled, keeping its window and its unchanged controls. A new interface that fails to start leaves the last working one on screen. Only debug builds accept an interface from `dev`: development interfaces are unsigned, so a release build refuses them.

Ctrl-C or `SIGTERM` stops `dev` cleanly, along with the app server and the desktop apps it opened.

Under that:

- `WasmHost::replace_guest` replaces a running guest while keeping the surface and the widgets it committed. The new guest's first commit is diffed against them.
- `NativeSurface::viewport` returns the viewport a surface lays out against.
- `leptoskit_axum::shutdown_signal`, also at `leptoskit::server::shutdown_signal`, resolves on Ctrl-C or `SIGTERM`, the signals `serve_listener` finishes its requests on.
- `leptoskit_codepush::dev` defines the development guest protocol, one long-polled `GET` per platform, with the `DevGuests` client and `follow_environment` for desktop runners behind the `http` feature. `leptoskit` exposes them through the new `codepush-http` feature.
- `leptoskit_bridge_ffi` adds `lk_dev_open`, `lk_dev_next`, `lk_dev_error`, and `lk_dev_destroy`. The generated iOS, macOS, and Android apps use them to follow `dev` without changes to their app-owned files.

Projects created before this release keep working. Their app-owned Linux and Windows runners open under `dev` but need a restart to show a change; to reload in place, copy the reload from a new project's `src/main.rs` and add `codepush-http` to the runner's `leptoskit` features.

`leptoskit native build ios` and `ios-simulator` remove `LD` and `LDPLUSPLUS` from Xcode's environment, so iOS builds work from a nix or devenv shell, which sets `LD=ld`.

- **Leave text inputs in widget tests with `blur`.** `WidgetTester::blur` moves focus out of a text input through its blur handler and pumps once, the way leaving a field does on every backend, so tests can drive validation that runs when the user leaves a field. Like `enter_text`, it rejects controls that are not text inputs. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #190](https://github.com/ifiokjr/leptoskit/pull/190)

#### Add `leptoskit_db_schema` for generated migrations

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #255](https://github.com/ifiokjr/leptoskit/pull/255)

The first piece of leptoskit's built-in database layer. A database schema (tables, typed columns, keys, unique and partial indexes, foreign keys, checks, comments, and namespaces) is described once and compared with the snapshot committed beside the previous migration, producing a step-by-step plan and the SQL that applies it, for Postgres and SQLite. Renames come from explicit hints, or from a question when a column or table appears while a similar one disappears, so data is never dropped by a guess. Every step says what it risks for existing rows, and steps that delete data are called out. SQLite migrations rebuild tables whenever `ALTER TABLE` can't make a change, keeping every row and checking every foreign key. Apps use it through the upcoming `leptoskit db` commands rather than directly.

#### Declare database tables as Rust structs

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #256](https://github.com/ifiokjr/leptoskit/pull/256)

`leptoskit_db` adds `#[db::table]`: one struct declares a table, its server-side row, the `New…` type an insert takes, and the `Public…` type a client may see, which leaves out every `private` and `secret` column. Rows have no serde impls and redact secrets in `Debug`, so they can't be returned from a server function by mistake; on the client the row type is empty, and no database code is compiled in. Columns declare rules once (`min_len`, `max_len`, `min`, `max`, `one_of`, `regex`) and get them as database checks, as `validate()` on the server and the client, and as JSON Schema for generated clients. `Id<T>` is a typed UUIDv7 key whose columns become foreign keys, `#[derive(db::Enum)]` stores enums as text, and every table registers its schema for the migrations `leptoskit db` will generate.

- **Query and write tables on Postgres and SQLite.** `leptoskit_db` connects with `Db::connect` to Postgres (`tokio-postgres`, TLS by default) or SQLite (`rusqlite`, bundled, the same SQLite as the on-device plugin), and every `#[db::table]` gets typed queries and writes: `User::query().filter(|u| u.karma.gt(10) & u.name.is_not_null()).order_by(|u| u.karma.desc()).limit(20).all(&db)`, `count`, `exists`, `find`, `get`, `NewUser { … }.insert(&db)`, `upsert` on any unique index, `save`, `delete`, bulk `update()`, and `db.transaction(async |tx| …)`, which commits on `Ok` and rolls back on `Err` or cancellation. Writes check the columns' rules first, timestamps fill themselves in, soft-deleted rows stay hidden until a query asks for them, and errors tell a taken unique value, a broken foreign key, and a failed check apart without putting SQL or values in their message. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #258](https://github.com/ifiokjr/leptoskit/pull/258)

#### Apply database migrations at startup

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #259](https://github.com/ifiokjr/leptoskit/pull/259)

`leptoskit::db::migrations!("../migrations")` embeds every migration listed in `migrations/registry.txt`, and `db.migrate(MIGRATIONS)` applies the pending ones, each in its own transaction, recording a checksum of its SQL. An applied migration that was edited afterwards, or one the build doesn't know, stops the server instead of being skipped. Servers starting together on Postgres take turns through an advisory lock, and SQLite migrations that rebuild tables run with foreign keys off and verified before they commit. `db::testing::sqlite()` gives tests an in-memory database with every table created from the code.

#### Generate migrations with `leptoskit db`

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #260](https://github.com/ifiokjr/leptoskit/pull/260)

A `[db]` table in `leptoskit.toml` turns on `leptoskit db`, which writes migrations from the app's `#[db::table]` structs instead of by hand. `leptoskit db migration create add_posts` reads the tables from the db crate (which calls `leptoskit::db::export_schema!()` once), compares them with the snapshot beside the latest migration, and writes the SQL, a new snapshot, and the registry entry. It asks whether a column that disappeared was renamed, refuses to drop data without `--allow-destructive`, and writes empty data migrations with `--empty`. `leptoskit db check` fails CI when the tables changed without a migration; `migrate`, `status`, and `reset` work against the database in `DATABASE_URL`. The `leptoskit` crate gains `db`, `db-server`, `db-postgres`, and `db-sqlite` features that expose `leptoskit::db`.

#### Scaffold a database with `leptoskit new --db`

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #261](https://github.com/ifiokjr/leptoskit/pull/261)

`leptoskit new my_app --db postgres` (or `--db sqlite`) creates a project that already uses the database: a `my_app_db` crate declaring an example `Note` table, an empty `migrations/` directory, a `[db]` table in `leptoskit.toml`, a server that applies pending migrations before serving, and a backend with `GET /api/notes` and `POST /api/notes`. Run `leptoskit db migration create init` and `leptoskit dev` to see it work. A new guide page, "The database", covers declaring tables, private and secret columns, queries, and migrations.

- **Declare custom scalars with `#[derive(db::Scalar)]`.** A newtype such as `struct Handle(String)` with `#[db(min_len = 4, max_len = 20, regex = "...", normalize(trim, lowercase))]` is now valid by construction. `Handle::parse` normalizes and checks it, `Deserialize` goes through `parse`, its JSON Schema carries the rules, and every column of its type gets them as database checks. Number scalars take `min` and `max`. `Email`, `Url` and `Slug` come built in. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #266](https://github.com/ifiokjr/leptoskit/pull/266)
- **Share columns across tables with `#[db::mixin]`.** Declare a set of columns once with `#[db::mixin]` and flatten it into any table with `#[db(flatten)]`, or `#[db(flatten, prefix = "billing_")]` to use it twice. The table gets the columns in place, along with the mixin's rules, indexes, and unique constraints. Public types nest each mixin's public type, and inserts take only what the database layer doesn't fill in. `db::Timestamps`, `db::SoftDelete` and `db::Versioned` come built in. `db::Versioned` adds optimistic concurrency: `save` refuses a row that changed since it was read, with `DbError::Stale`, and the new `#[db(version)]` marker does the same for any integer column. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #267](https://github.com/ifiokjr/leptoskit/pull/267)
- **Follow links between tables.** Every `Id<T>` column now gets two methods: an accessor that loads the row it points at, so `author_id` gives `post.author(&db)` (`Option<T>` for an optional link), and a `Post::by_author(id)` query. `#[db(backlink = posts)]` adds the reverse to the linked table: `user.posts()` returns a query that filters, orders, and counts like any other. Tables keyed by `Id<Self>` implement the new `HasId` trait. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #269](https://github.com/ifiokjr/leptoskit/pull/269)
- **Cascade soft deletes, and check links at commit.** `#[db(on_soft_delete = cascade)]` on a link soft-deletes its rows when the row it points at is soft-deleted, then follows their own cascading links, in one transaction that joins yours if you're already in one. Single and bulk deletes both cascade, and rows already deleted keep their original time. `#[db(on_delete = deferred_restrict)]` checks a link when the transaction commits, so related rows can be inserted in any order. A soft delete now also moves a versioned row's version. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #270](https://github.com/ifiokjr/leptoskit/pull/270)
- **Access policies on tables.** `#[db::policy(select, |article, cx| …)]` below `#[db::table]` declares who may read, insert, update, or delete rows, as a condition that can read the request's globals: any value in a `Context`, found by type, such as `cx.global(|viewer: &Viewer| article.author_id.eq(viewer.0))`. Once a table has a policy, only what a rule allows is allowed, and a `deny` wins. Reads, updates, and deletes skip rows the request may not touch. A refused insert, or an update whose result isn't allowed, fails with `DbError::Forbidden` and the rule's message, and nothing is written. `db.with_context(cx)` scopes a handle to a request. A plain `Db` acts for an empty context, and `db.system("reason")` bypasses policies explicitly. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #271](https://github.com/ifiokjr/leptoskit/pull/271)

#### Schema packs: tables that libraries ship

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #273](https://github.com/ifiokjr/leptoskit/pull/273)

A library declares its tables with `#[db::table(…, pack = "auth")]`, and an app adds it with `leptoskit::db::packs!(leptoskit_auth)`. Pack tables join the app's schema and migrations in the pack's own namespace, and the app reads them and links to them. Only the pack's crate writes them, since their generated write methods are private to it. Exporting the schema checks that every pack table works on both Postgres and SQLite, with no raw SQL and no links outside the pack.

#### Add ARIA accessibility across every platform

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #141](https://github.com/ifiokjr/leptoskit/pull/141)

Every element now carries ARIA-shaped props — accessible names and descriptions (with id references), roles, states, range values, live regions, modal flags, set positions, orientation, tab order, and a passthrough for any other `aria-*` attribute — rendered as real attributes and semantic HTML elements on the web (headings, lists, landmarks, `<hr>` separators) and mapped onto each platform's accessibility API: VoiceOver traits on iOS, NSAccessibility roles and announcements on macOS, TalkBack class names and live regions on Android, MSAA roles with UIA control types and notifications on Windows, and GTK accessible properties, states, and relations.

Built-in components carry their semantics: Heading, Status, and Alert join the vocabulary; dividers are separators; lists, tiles, the app bar banner, and the scaffold main landmark announce their structure; activity indicators are named; and controls with a declared control role join the tab order and activate from Enter and Space. Validation rejects nameless text inputs and pressables, duplicate or dangling relationship ids, named presentation, and passthrough attributes that shadow typed builders.

The widget wire stays additive: `accessibility_label` keeps its key, unknown roles decode to none, and the resolver folds labelled-by and described-by references into concrete labels for native backends, numbers radio options within their group, and fills live-region announcement text. The widget tester gains `Finder::aria_role` and a semantics tree dump; the AppKit, browser, and widget suites cover the mapping, and the gallery demonstrates headings and a typing-indicator status region.

Focus scopes, traps, and custom accessibility actions remain pending; runtime services such as announcements outside live regions, screen-reader detection, and reduce-motion belong in a follow-up accessibility runtime plugin.

#### Retain application services through asynchronous shutdown

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #164](https://github.com/ifiokjr/leptoskit/pull/164)

`App` can defer one executor-neutral service until native wake and close owners exist. Services borrow the actual interpreter briefly, preserve unrelated requests and cancellations, and receive coalesced notifications from arbitrary threads.

Rust and C close handles stop admission and delivery before teardown, retain the interpreter until owned work has joined, and return cancelled close awaits to the configured owner-thread scheduler. Embedders must retain and poll closing owners until completion. Generated Swift, Kotlin, JNI and CLI service wiring remains separate work.

#### Add scoped asynchronous native plugin execution

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #68](https://github.com/ifiokjr/leptoskit/pull/68)

Generated iOS, Android and macOS hosts can install HTTP and WebSocket plugins through validated, host-owned origin scopes. HTTP exposes bounded binary requests/replies; WebSocket exposes owned connection handles, bounded send/receive operations and explicit close. Redirects, destination scopes, deadlines and transport errors retain their documented platform behavior.

Add optional `async-host` dispatch with bounded request/reply reservations, cancellation, permission-revocation handling and resource cleanup. Native callbacks return owned replies to the bridge thread. Cancelled work retains its native job budget until cleanup completes. Unread WebSocket-open replies retain provisional resource ownership, so cancellation or teardown cannot orphan sockets.

The CLI enables async features only for selected native plugins. Interpreter clients use `default-features = false` and `guest`; browser clients retain their browser adapters. The default bridge has no Tokio runtime dependency. CI explicitly exercises async host and loopback network tests, Swift callback lifecycle, the ignored Kotlin/JNI lifecycle fixture under JVM checking, and guest dependency isolation.

#### Sign-in: the auth pack, sessions, and email codes

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #288](https://github.com/ifiokjr/leptoskit/pull/288)

`leptoskit::auth` (the `auth` feature, with `db-postgres` or `db-sqlite`) adds sign-in in the app's own database.

- **Tables:** accounts, verified emails, sessions, single-use challenges, rate limits and an audit trail ship as the `auth` schema pack. Policies show each person only their own rows.
- **Email codes:** one flow signs in to an address's account or creates it, and gives the same answer for every address. Codes are keyed hashes that expire after 10 minutes, work once, and allow five guesses.
- **Sessions:** opaque 256-bit tokens, of which only the hash is stored. They have idle and absolute lifetimes, and support sign-out, revoking one or all other sessions, rotation, and disabling an account.
- **Rate limits:** sending and checking codes is limited per address and per IP address.
- **Policies:** `cx.signed_in(|me| post.author_id.eq(me.user_id()))` lets access policies read the signed-in person.
- **Sign-up:** `OnSignUp` creates the app's user row in the same transaction as the account.

#### Sign-in over HTTP

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #289](https://github.com/ifiokjr/leptoskit/pull/289)

With the `server` feature, `leptoskit::auth` serves sign-in.

- **`auth.router()`** adds JSON routes under `/auth`. They send and check email codes, report the current session, sign out, and list and revoke a person's sessions.
- **`auth.layer()`** resolves every request's session, from an `HttpOnly` `__Host-session` cookie on the web or a bearer token from a native app. It hands handlers a database that acts for the signed-in person.
- **Other sites** can't make state-changing requests: the layer refuses them by checking `Origin` and `Sec-Fetch-Site`.
- **Handlers** take `auth::SignedIn` (`401` when signed out). Server functions call `auth::require_signed_in()` and `auth::request_db()`.
- **Development:** `.dev_mailbox()` shows outgoing email at `/_leptoskit/mail`, and `.dev_sign_in()` signs in to any address without a code. Neither exists in release builds.

#### Sign-in email and `[auth]` configuration

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #291](https://github.com/ifiokjr/leptoskit/pull/291)

Sign-in is configured in `leptoskit.toml` under `[auth]`. Secrets aren't written there: each setting that needs one names the environment variable that holds it. `Auth::from_config` builds the service from that configuration, and `LeptoskitConfig::from_toml` lets a server embed its configuration, so it needs no file at runtime.

Email goes out over SMTP (`auth-smtp`), or as a JSON webhook signed per Standard Webhooks (`auth-webhook`) that your endpoint passes to any provider. With neither configured, development keeps email in the mailbox at `/_leptoskit/mail`, and a release build refuses to start.

#### Split lazy web code by default

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #155](https://github.com/ifiokjr/leptoskit/pull/155)

Lazy functions, routes, and islands now build into on-demand wasm chunks without setting `bundle.split`. Clients without lazy code skip splitting and keep their existing binding cache. Explicit `split = false` remains available and rejects lazy imports before writing an unusable client. Environment rustflags only require relocations when the client actually contains lazy code.

Standalone clients discard linker metadata before optimization so Binaryen can produce the same small bundles as an eager build. Application custom sections are preserved, and existing binding caches are refreshed.

The native CodePush investigation includes a reproducible wasmi probe and a proposed shared API; native chunk loading is not yet a supported application feature.

#### Show buttons as pressed while they are held

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #201](https://github.com/ifiokjr/leptoskit/pull/201)

A styled button showed no change while you pressed it, on the web and on every native platform. `Button` and `Pressable` now accept `active:` utilities, such as `active:bg-indigo-700 active:opacity-80`, and apply them while the control is held down: through CSS `:active` on the web, and through each platform's own pressed state on macOS, iOS, Android, Linux, and Windows. The web adapter's rendered root now listens passively for touches, because iOS Safari applies `:active` to a touched element only where the page listens for them. Only the held control is restyled, in place, keeping its accessible name and role. `active:` on any other element is a `ClassError`, since native platforms report presses only for controls.

Controls without `active:` utilities now dim to 70 percent of their opacity while held: every `Pressable`, and every `Button` with its own background, border, or corner radius. Buttons that keep their native chrome keep the platform's own highlight instead. The Windows backend no longer darkens held pressables, or buttons with their own background, border, or corners, on top of that, and Android buttons in non-scrolling containers show their pressed state on touch down rather than after the tap delay.

Backends report holds as the new `NativeEvent::PressIn` and `NativeEvent::PressOut`. `WasmHost::dispatch` handles them on the host without entering the guest, and other hosts pass them to the new `NativeSurface::press_in` and `NativeSurface::press_out`. `StyleContext::active` resolves a tree with its controls pressed, `lk_app_event` accepts kinds 8 and 9 for the generated apps' renderers, and `WidgetTester::hold` and `release` let widget tests check pressed styles. HarmonyOS buttons keep ArkUI's own press effect and don't apply `active:` yet.

#### Make code push deltas pay off and harden the update paths

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #204](https://github.com/ifiokjr/leptoskit/pull/204)

Deltas used to be computed between gzip streams, which differ in every byte after the first change, and stored inserted bytes as a pretty-printed JSON array, so a delta was usually larger than the full download. Containers now compress each file in chunks of about 64 KiB that come out the same whenever their input does, so a delta carries only the chunks an update changed; inserted bytes travel as base64 in compact JSON; and `release` keeps a delta only when it is at most three quarters of the full container. Containers stay ordinary gzip, so older clients read them unchanged; a delta in the new encoding fails to parse on an older client, which falls back to the full download. Chunked compression makes a container up to a few percent larger than plain gzip.

A release that failed activation on a device is no longer downloaded again on every check: while the channel still names it, `InstalledBundles::sync` reads only `index.json` and reports the working release as current, instead of downloading up to 32 MiB and failing.

`CodePushClient::with_max_container_bytes` now bounds full downloads and delta bodies as well as delta output, and `CodePushClient::sync` replaces its staging directory whole, so files of the previous release no longer survive an update and a failed sync leaves the previous staging intact.

The release store assembles each release directory, container, signature, and delta, under a temporary name and renames it into place, so a crash can no longer leave an unsigned release that blocks re-releasing that version. `pull` checks the bucket's container against the signed manifest before decompressing anything, and containers with bytes after their last file, or after a file's gzip stream, are rejected. The code push routes log the cause of every `500` through `tracing`, while the client still sees only a fixed message.

#### Trial and roll back installed native bundles

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #70](https://github.com/ifiokjr/leptoskit/pull/70)

`leptoskit_codepush::installed::InstalledBundles` owns the release a native shell runs. `sync_with` downloads and verifies an update and stores the signed container without running it. It also refuses bundles whose `runtime.json` names a different native ABI or shell contract, or a plugin the shell lacks. `launch` marks a downloaded release as a trial before returning its Wasm. `confirm` promotes it once the host has rendered it, and `reject` drops it and keeps the last working release. A trial left unconfirmed by process death is rejected on the next open. Confirmed releases load offline and have their signatures checked again on every launch. A newer store baseline supersedes older downloads.

The `http` feature adds a blocking `HttpTransport` for a serialized background worker. It requires HTTPS unless plaintext is explicitly allowed, follows no redirects, caps responses at 32 MiB, and gives each request 30 seconds. `CodePushClient::download_verified` authenticates an update without staging it or advancing the running release, and `bundle_directory_as` packs a store baseline with an explicit identity.

- **Pull the newest code push release from a bucket.** `leptoskit codepush pull` copies a channel's newest release from an S3-compatible bucket into the release store, so a CI runner that starts empty can number the next build and compute its delta from the release devices run. Before anything is written, the release has to be signed by a key in `codepush.public_keys` and its container has to match the signed manifest. `pull_channel` and `ReleaseStore::import_release` expose the same thing to Rust callers. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #25](https://github.com/ifiokjr/leptoskit/pull/25)

#### Sign code push releases

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #16](https://github.com/ifiokjr/leptoskit/pull/16)

`leptoskit codepush release` signs every release with an ed25519 publisher key, read from `--signing-key <PATH>` or the `LEPTOSKIT_CODEPUSH_SIGNING_KEY` environment variable. `leptoskit codepush keygen` creates the key and prints the public key for the new `codepush.public_keys` setting, which is required when code push is enabled.

`CodePushClient::new` takes a `TrustedKeys` set. Before downloading anything, the client rejects offers that are unsigned, signed by an untrusted key, tampered with after signing, meant for another app or channel, or not newer than the release it runs, so a compromised registry can neither ship its own code nor roll devices back. `ReleaseStore::ingest_directory` takes the signing key, and update offers carry the detached signature.

#### Serve code push from any static host

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #17](https://github.com/ifiokjr/leptoskit/pull/17)

Releases now live in a static layout that any object store or CDN can serve with no server running. Each channel has one `index.json`, which holds the newest release's signed manifest. Every release file is immutable, and deltas are addressed by the SHA-256 of the container they apply to. `CodePushClient` reads the index, verifies it, and asks for the delta from its exact container. A 404 sends it to the full download. `leptoskit codepush serve` serves the same files, and the older `check`, `download`, and `delta` endpoints keep working.

`leptoskit codepush publish` uploads a channel to an S3-compatible bucket (Backblaze B2, Cloudflare R2, AWS S3, MinIO), writing `index.json` last so devices never see a release before its files. Credentials come only from `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`, and `AWS_SESSION_TOKEN`. Publishing refuses to roll a channel back or replace a release the bucket already holds with different contents.

Breaking: `ReleaseStore::ingest_directory` returns an `IngestedRelease`, `ReleaseRecord` no longer carries `delta_from_previous` (use `ReleaseStore::delta`), and a `Transport` must report a 404 as `CodePushError::NotFound`.

#### Add editor schemas and explicit configuration migrations

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #146](https://github.com/ifiokjr/leptoskit/pull/146)

Generated `leptoskit.toml` files include a pinned JSON Schema directive and `schema_version = 0` for editor autocomplete and diagnostics. The docs publish `schema.json`, `schemas/leptoskit.schema.json`, and the major-pinned `schemas/leptoskit.v0.schema.json`, generated from Rust types and checked for drift in CI. Version 0 is unstable; breaking changes are expected until the first version 1 schema.

Run `leptoskit migrate --dry-run` to preview an existing file, `--check` to detect pending migration, or `leptoskit migrate` to apply it. The first migration stamps unversioned files and moves `build.wasm_opt` to `bundle.wasm_opt` while preserving comments. Invalid settings, conflicting keys, and future versions fail without changing the file. Existing unversioned configurations remain readable.

- **Add direct asynchronous desktop file/directory/save/message dialogs and bounded browser file selection.** Add explicitly configured per-user autostart registration and process-lifetime single-instance guards on desktop. Dialog guest adapters, mobile document providers and second-launch activation/argument handoff remain unsupported. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #54](https://github.com/ifiokjr/leptoskit/pull/54)

#### Control every attached device from the CLI, JSON, or MCP

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #225](https://github.com/ifiokjr/leptoskit/pull/225)

`leptoskit device` drives what is attached to the machine: iOS simulators and devices, Android phones and emulators, native desktop apps, and websites. `device list` reports the inventory; one-command actions print JSON outcomes; `device run` takes a whole control document — a target and ordered steps — from a file or standard input. Driver steps run through W3C sessions (Appium for native targets, a spawned chromedriver/geckodriver/safaridriver for websites), and device steps shell out to `simctl`, `devicectl`, and `adb` directly, so screenshots, app management, input injection, boot, and logs work with no driver server running at all.

`leptoskit mcp serve` exposes the same API as an MCP stdio server with three tools — `list_devices`, `control` (the document's JSON Schema rides along), and `screenshot` (image content inline) — and `leptoskit mcp http` accepts identical JSON-RPC messages as POST bodies, so agents can drive devices through an MCP client or a plain curl. The document schema is published at `schemas/control.schema.json` and generated by `devenv shell schema:update`.

#### Route apps by their files

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #105](https://github.com/ifiokjr/leptoskit/pull/105)

Put one file per screen in `src/routes/`, add a `[router]` table to `leptoskit.toml`, and leptoskit writes `src/routes.rs`: a module for every route file and one `Route` enum that parses URLs and writes hrefs. The file names follow Expo Router: `index.rs`, `[note].rs`, `[[lang]]/`, `[...topic].rs`, `[[...topic]].rs`, `(group)/`, `_layout.rs`, `_private.rs`, `+not-found.rs` and `+html.rs`. A route with parameters declares a `#[derive(Route)]` struct whose fields come from the path when they are named after a segment and from the query string otherwise, so a link to a missing route, or one without its parameters, does not compile. Pages are a lowercase `page` function or a capitalised `#[component] Page`.

`leptoskit routes` writes the module and lists the routes, and `--check` fails in CI when the committed module is out of date. `dev`, `build` and `native` regenerate it before compiling, writing only when the routes changed. The generator reports every problem with its file and line, and a route struct that drifts from its file name is a compile error even before the generator runs again. The new crates are `leptoskit_router`, `leptoskit_router_macros` and `leptoskit_router_codegen`; `examples/file_routes` is a complete app.

#### Know how a foldable phone is folded

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #212](https://github.com/ifiokjr/leptoskit/pull/212)

The new `posture` runtime plugin, `leptoskit_plugin_posture`, reports whether a foldable is closed on its cover screen, half open, or open flat, with every hinge that crosses the app's window: its axis, its position as fractions of the window, its angle, and whether it hides content. `Posture::panes` splits the window around the folds for percentage layouts, and `is_tabletop` and `is_book` name the two half-open postures. `leptoskit_use::use_posture` turns a posture signal into memoized queries.

iOS 27.1 reads the iPhone Duo's hinge through `UIHingeInteraction` and the fold's position through the view's division reserved region. Android reads Jetpack WindowManager's folding features and the hinge angle sensor, so book folds, flips, and trifolds report each fold. Browsers read the Device Posture and Viewport Segments APIs.

Generated Android apps now also handle `screenLayout` and `smallestScreenSize` configuration changes. Before, folding or unfolding a foldable recreated the activity and restarted the guest, losing the app's state. Apps generated earlier should add `screenLayout|smallestScreenSize` to `android:configChanges` in `android/app/src/main/AndroidManifest.xml`.

A guest that fails to start on iOS or macOS now shows its error. The message label was sized from the view's bounds at the moment of failure, which are still empty during launch, so the error was laid out off screen and the app showed a blank window.

- **Expose host-assigned guest plugin and call identities for application-owned event routing.** Call identities remain available until the response future consumes its result or cancels its outstanding call. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #166](https://github.com/ifiokjr/leptoskit/pull/166)

#### Fold posture and ArkTS plugins on HarmonyOS

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #214](https://github.com/ifiokjr/leptoskit/pull/214)

HarmonyOS apps can now select runtime plugins written in ArkTS, and `posture` is the first. A plugin crate declares `harmonyos-type` (and optionally `harmonyos-context`) and ships `harmonyos/<Type>.ts`, a class implementing the host's `NativePlugin` interface. Every prepare regenerates `entry/src/main/ets/leptoskit` with the host's plugin contract, the selected plugins, and a registrant. The Node-API runtime gains `register` and `complete`, and each render batch now carries the guest's plugin requests and cancellations. The page hands those to the plugins and returns each answer on a later UI turn. A call the guest stops awaiting cancels its plugin work and drops the answer.

On HarmonyOS, `posture` reads `@ohos.display`. The fold status gives each hinge's state, including the API 15 statuses for both hinges of a trifold. `getCurrentFoldCreaseRegion` gives where each fold crosses the window, and `foldAngleChange` (API 12) gives each hinge's angle. Flip phones answer `isFoldable()` with false, so the plugin reports them as unknown rather than rigid. This has not yet run on a HarmonyOS emulator or device.

Projects created earlier keep their own `Index.ets` and `index.d.ts`. Copy both from a newly created starter to run ArkTS plugins.

#### Add an experimental native HarmonyOS deployment target

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #129](https://github.com/ifiokjr/leptoskit/pull/129)

Generate an ArkUI counter with `leptoskit new app --platform native --native-target harmonyos`. Prepare or package arm64 and x86-64 HarmonyOS hosts with `native prepare/build harmonyos` or `harmonyos-x86_64`, using the DevEco native toolchain and Hvigor to produce a HAP. The host embeds the existing Wasm interpreter and renders native ArkUI widgets. It rejects unsupported platform/worker plugins and code push. Both architectures have been packaged with the public OpenHarmony SDK; Huawei SDK and device validation remain required before production use.

Generated apps include a Hypium/UiTest counter journey and instructions for DevEco emulator/device instrumentation, SDK-free checks, and failure artifacts.

A dedicated HarmonyOS CI job runs guest and render tests and builds real app and instrumentation HAPs for arm64 debug and x86-64 release with the pinned public SDK. Device UI execution remains a separate validation step.

#### Add document import, orientation and sensor capabilities

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #64](https://github.com/ifiokjr/leptoskit/pull/64)

Add independent `document_picker`, `screen_orientation`, `light_sensor` and `pedometer` crates with selected-plugin native registration and typed binary guest contracts.

Document picker imports one ordinary file through iOS, macOS, Android or a browser, with bounded off-thread native reads, balanced resource access and shared Rust reply storage. Windows/Linux hosts can use an explicit parent-window API. Directory selection, export, multiple files and persistent grants remain separate work.

Screen orientation reports the presented interface and holds a cancellable policy lease for the owning iOS scene, Android Activity or browser document. Cancellation and lifecycle cleanup release the request; a pending lease does not prove that the operating system rotated the interface. Native desktop locking is unavailable.

Light sensor acquires one ambient-illuminance sample on Android or supported browsers. Pedometer preserves the distinction between iOS interval counts and Android cumulative boot counters, supports explicit permission requests, and bounds iOS historical queries. Pedometer purpose strings and Android activity-recognition permission remain application-owned. These ports do not claim physical sensor accuracy or verified access to real activity history.

Extend generated hosts with weak Apple presentation context and an Android activity-result broker that retains presentation ownership through cancellation and rejects stale results. Generated Android Activities handle orientation configuration changes without recreating the guest. Extend browser and Android integration fixtures with synthetic document, light-sensor, orientation and broker lifecycle checks.

- **Add leptoskit_aria: headless accessible components.** New `leptoskit_aria` crate with the widget-behavior layer over the declarative ARIA model: `DisclosureButton`/`DisclosurePanel`, `ToggleButton` (including mixed state), `Tabs` building a tab list, tabs, and panels whose selection and set position cannot disagree, `Field` label/help/error wiring for any input, and `Dialog` building a container named by its title and described by its description. Components emit the shared element tree, so one widget renders with correct semantics on the web and every native backend, and the renderer's validation rejects half-wired trees. Tabs stay in the tab order and dialogs are not marked modal until the renderer gains key events and focus containment. Re-exported behind the `leptoskit` `aria` feature and documented in the new accessible-components guide. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #179](https://github.com/ifiokjr/leptoskit/pull/179)

#### Add `leptoskit_docs`, Markdown with leptos components

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #136](https://github.com/ifiokjr/leptoskit/pull/136)

A new crate for writing documentation the way MDX does, with leptos in place of React: a capitalised tag in a Markdown file, such as `<Callout kind="tip">`, renders a leptos component registered by name, and the Markdown between its tags becomes the component's children. Lowercase tags are plain HTML, and GitHub alerts (`> [!TIP]`) render as callouts.

`Site` builds a whole book, laid out by an mdBook-style `SUMMARY.md`, into a static site: chapter navigation, an on-page table of contents, search, build-time syntax highlighting with tree-sitter grammars, light and dark themes, and copy buttons. Every link between pages and headings is checked, so a broken link fails the build instead of shipping. `Site::serve` rebuilds on every edit and reloads open pages.

- **Add leptoskit_form: headless form state.** New `leptoskit_form` crate porting TanStack Form's core loop: declared `FieldSpec`s (label, help, initial value, `required`, ordered sync validators over the entered text) and a `FormState` whose edits arrive as `for_change` and `for_blur` proposals. A field's error first appears when the user leaves the field or submits the form, then re-resolves live on every edit, so a fix clears it at once and typing never raises one mid-word. `submit` runs every rule and returns the values or the error-filled state. Rendering goes through `leptoskit_aria::Field` wiring (`field_wiring`, `input`, `submit_button`), unstyled: ids are prefixed with the form's id so forms with the same field ids share a screen, labels name inputs, help and error ride `aria-describedby`, and values turn `aria-invalid` — announced by screen readers on the web and every native backend. A form rejects duplicate field ids. Re-exported behind the `leptoskit` `form` feature, which implies `aria`, and documented in a new forms guide. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #190](https://github.com/ifiokjr/leptoskit/pull/190)

#### Format Rust and view!

markup with `leptoskit format`

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #108](https://github.com/ifiokjr/leptoskit/pull/108)

A new `leptoskit_fmt` crate, built into the CLI as `leptoskit format` (and `lk format`), replaces leptosfmt. It formats Rust with rustfmt — nightly by default, honouring the project's `rustfmt.toml` — and lays out the rstml markup inside `view!` macros (`view`, `leptos::view`, and `leptoskit::view` by default) at the same width and indentation, with every Rust expression inside the markup formatted by rustfmt at the column it sits in. Formatting is idempotent, keeps every comment, copies text and literals byte for byte, and leaves markup that does not parse exactly as written with a warning.

`--check` prints a diff and fails when anything would change, `--stdin` formats for editors, and whole-workspace runs format files in parallel and skip files a previous run left formatted. Settings live in a new `[format]` table in `leptoskit.toml`.

- **Add leptoskit_table: headless tables.** New `leptoskit_table` crate porting TanStack Table's core model: typed `Column`s (cell reader, optional row comparator, optional filter rule), `TableState` with pure reducers (`for_header_press` cycling none→ascending→descending→none, `for_filter`, `for_page` with clamping), and a `rows` pipeline that filters, stably sorts, and paginates, reading each text sort key once. A page size of zero turns pagination off, and a page past the end of shrunk data shows the last page. `DataTable(&columns, &data, &state, &labels, on_change)` renders the grid — `columnheader` cells holding real sort buttons, `aria-sort` on the sorted one, `row`/`cell` roles, a filter field, and a pagination footer with a live page status — over the shared element tree, and hands every interaction back as the whole next `TableState`. `TableLabels` translates its words. Re-exported behind the `leptoskit` `table` feature. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #189](https://github.com/ifiokjr/leptoskit/pull/189)
- **Add cross-platform reactive utilities.** Add `leptoskit_use` and the optional `leptoskit::hooks` re-export. State, derived values, watchers, timer filters, typed storage, and viewport breakpoints use Leptos signals without requiring a DOM or executor. Host clocks, viewport signals, and storage adapters make environmental dependencies explicit. CI enforces 100% line coverage for the new crate and checks browser WASM and WASI builds. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #27](https://github.com/ifiokjr/leptoskit/pull/27)
- **Add leptoskit_virtual: headless list virtualization.** New `leptoskit_virtual` crate porting TanStack Virtual's model to the shared element tree: a `Virtualizer` with fixed or per-index size estimates, measured sizes that shift everything after them (and re-resolve only those items), overscan, and clamped aligned scroll targets (`Align::Auto/Start/Center/End`). `VirtualList(&virtualizer, Axis::Vertical, on_scroll, item)` renders the window inside a `ScrollView` sized to the real total: each item sits in a `listitem` slot sized to it along the axis and stretched across it, carrying `aria-posinset` and `aria-setsize` for its place in the whole list. Invalid estimates and measurements are errors naming the index — including from `offset_for_index`, through `OffsetError` — never silently clamped offsets, and an offset past the end of a list that shrank resolves to its last window. Re-exported behind the `leptoskit` `virtual` feature. The generated macOS, iOS, and Android apps do not report scroll events yet, so there the window stays on its first page. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #187](https://github.com/ifiokjr/leptoskit/pull/187)
- **Ship `lk` as the CLI's alias binary.** `cargo install leptoskit_cli` now installs `lk` next to `leptoskit`: the same entry point built under a shorter name, so `lk new`, `lk dev`, and `lk build` all work. Help and error messages answer to the alias, completion scripts complete the invoked name (`lk completions zsh > ~/.zfunc/_lk`), and the scaffold's next-step hint echoes it. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #19](https://github.com/ifiokjr/leptoskit/pull/19)

#### Add image transformation and capture-policy plugins

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #63](https://github.com/ifiokjr/leptoskit/pull/63)

Add a separate image manipulation crate with bounded JPEG/PNG decoding, ordered crop, resize, rotation and flip operations, and encoding. Native hosts run transforms on a Rust worker; browser applications can call the same engine. Input, output, dimensions and transformation work have explicit limits. Decoder-private allocations remain subject to codec behavior. This crate requires Rust 1.88.

Add a screen capture crate with Android window-owned secure-flag leases, Android 14 screenshot notifications, iOS scene recording-state observation and app-wide screenshot notifications. Cancellation and teardown release subscriptions and owned leases. Capture prevention is unavailable on iOS; desktop and browser capture capabilities are unavailable. The plugin never captures pixels.

#### Code push the Wasm guest of generated iOS and Android apps

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #71](https://github.com/ifiokjr/leptoskit/pull/71)

Generated native apps can now download and activate a replacement Wasm guest. With `codepush.enabled`, the host shows an update bar. **Check for update** downloads a release on a background worker and verifies its signature, target, and version. It also checks the release's `runtime.json` against the installed shell's ABI, `native.update_contract`, and plugins. **Activate update** runs the release as a trial beside the working guest. It is confirmed only after its native root attaches. A release that traps or renders nothing is rejected, and the working guest keeps running. A trial interrupted by process death is rejected on the next launch, and confirmed releases load offline with their signatures checked again.

`leptoskit native bundle` builds the guest and its `runtime.json` for `leptoskit codepush release`, and `native prepare` embeds the guest as the app's trusted store baseline, together with its update configuration. `leptoskit_bridge_ffi` exposes `InstalledBundles` to Swift and Kotlin as `lk_updates_*`, and each host drives it from one background worker. Release builds require HTTPS; `codepush.allow_http` permits a plaintext registry in debug builds only.

A `mobile_codepush` example runs the whole journey on a simulator or emulator, and CI runs it on both.

#### Render shared primitives with Android Views

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #67](https://github.com/ifiokjr/leptoskit/pull/67)

Add `leptoskit_android` and the `leptoskit` facade's `android` feature. Compile the crate's `android/WidgetHost.kt` into an Android application and construct `AndroidBackend` from it on the UI thread. Every existing native primitive renders a real Android view with exact Yoga outer frames in dp, `StaticLayout` text measurement that matches the drawn text, controlled inputs, native accessibility, RTL, scrolling, and lifecycle support. Android 9 (API 28) or newer is required.

CI renders every primitive on an Android emulator and fails unless the probe Activity reports a verified result.

#### Generate working iOS and Android applications

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #44](https://github.com/ifiokjr/leptoskit/pull/44)

`leptoskit generate app --platform native` and `leptoskit new --platform native` now create UIKit and Android applications that load a Rust/Wasm interface and exercise Rust and native plugins. `leptoskit native prepare` generates host build state and plugin registrants from selected direct Cargo dependencies; `leptoskit native build` produces an iOS bundle or Android APK.

The app owns its platform sources, manifest and device tests. Generated state stays under `.leptoskit`. Platform dispatchers cancel pending work when the guest closes. Android supports arm64 and x86-64 targets with a pinned Gradle wrapper. Swift and Kotlin linting includes the hosts and templates.

The old Apple executable skeleton is replaced by mobile app projects. Render transactions currently use JSON; plugin transport remains binary. Desktop scaffolding, full widget coverage and the remaining plugin catalogs are still under development.

#### Render shared primitives with GTK4 on Linux

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #60](https://github.com/ifiokjr/leptoskit/pull/60)

Add `leptoskit_gtk` and the `leptoskit` facade's `gtk` feature. The backend renders every existing native primitive with real GTK widgets, exact Yoga outer frames, controlled inputs, platform text measurement, styling, accessibility, scrolling, and lifecycle support. GTK 4.6 or newer is required.

Linux workspace tests now run real GUI checks under D-Bus and Xvfb when no display is available. GUI tests are serialized to preserve native keyboard focus. The development environment includes GTK and the native verification tools.

- **Add native Rive vector playback.** Add thread-confined native playback to `leptoskit_rive`, a pinned offline C++ runtime, and a shared `Rive` element with typed input updates. Native hosts render vector frames into platform image widgets on macOS, iOS, Android, GTK, Windows, and experimental HarmonyOS. Frames stay outside Wasm guest memory and UI JSON; idle, hidden, inactive, and reduced-motion scenes stop scheduling frames. Native playback rejects unsupported asset features and bounds encoded files, player counts, raster dimensions, and input bindings. Text, images, scripts, audio, layout, and view-model data binding are not supported by the native renderer. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #240](https://github.com/ifiokjr/leptoskit/pull/240)
- **Add application identity, localization snapshots, network state/change observation, validated outbound links and camera/microphone permission flows as separate plugin crates.** Register Apple and Android handlers from metadata, preserve unknown platform fields, and retain permission-request identity across Android Activity recreation. Incoming deep links and additional permission families remain unsupported. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #52](https://github.com/ifiokjr/leptoskit/pull/52)

#### Refresh native text metrics and carry writing direction

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #58](https://github.com/ifiokjr/leptoskit/pull/58)

Add `NativeSurface::refresh_text_measurer()` so hosts can replace immutable font metrics after DPI or text-scale changes without recreating controls. Pass each node's layout direction through native props and text measurement, and resolve AppKit leading/trailing alignment against that direction.

`TextStyle` includes a `direction` field. Use `..TextStyle::default()` when constructing a style that should keep the default left-to-right direction.

#### Render shared primitives with UIKit on iOS

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #69](https://github.com/ifiokjr/leptoskit/pull/69)

Add `leptoskit_uikit` and the `leptoskit` facade's `uikit` feature. `UIKitBackend` renders every existing native primitive as real UIKit views with exact Yoga frames, Core Text measurement, controlled inputs, VoiceOver traits and values, scrolling, and animation lifecycle support. `UIKitHost` retains one surface through a native view controller and forwards logical bounds and rotation. iOS 15 is the minimum.

CI renders every primitive in a signed UIApplication on a fresh simulator and fails on any native assertion.

- **Render shared primitives with Win32 controls on Windows.** Add `leptoskit_windows` and the `leptoskit` facade's `windows` feature. The backend renders every existing native primitive with real Win32 windows and standard controls, exact Yoga frames, grouped opacity, per-monitor DPI, controlled inputs, GDI text measurement, MSAA and UI Automation accessibility, scrolling, and lifecycle support. Executables must embed the crate's `app.manifest` to enable version 6 common controls. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #61](https://github.com/ifiokjr/leptoskit/pull/61)
- **Add app-owned directory factories, retained Rust workers, bounded native job accounting through actual cancellation completion, shared native support discovery and Android manifest contributions.** Generate runnable macOS AppKit applications alongside iOS and Android; exhaustive matches on `native::Target` must handle `Macos`. Add separate haptics, battery, device, secure-store and private-filesystem crates and exercise their supported operations in generated sample applications. Their platform implementations expose explicit unknown/unavailable states and do not claim full Expo or Tauri parity. Running synchronous writes may finish after cancellation. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #50](https://github.com/ifiokjr/leptoskit/pull/50)
- **Nightly signal call syntax.** Add a `nightly` cargo feature mirroring Leptos's own: on a nightly compiler, signals gained call syntax — `value()` reads, `set_value(true)` writes — across `leptoskit_use` hooks, `leptoskit_rpc` queries and mutations, `leptoskit_query` state, and the Leptos stack the toolkit forwards to (`reactive_graph/nightly`, `leptos/nightly`). Signals also compose wherever closures are accepted, and `MaybeRwSignal` itself becomes callable. The feature is a no-op on stable toolchains, so `--all-features` builds, CI, and publishing are unaffected; a dedicated CI job compiles and tests the feature on a nightly toolchain. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #133](https://github.com/ifiokjr/leptoskit/pull/133)

#### Keep composition helpers optional

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #57](https://github.com/ifiokjr/leptoskit/pull/57)

Move the 31 layout and presentation builders into `components::helpers`, leaving native primitives as the primary `components` and prelude imports. Existing helper users should import the functions they need from `leptoskit::components::helpers` or `leptoskit_native::components::helpers`.

Build the shared component studio from `View`, Tailwind, and native controls. Document the React Native and Lynx primitive mappings separately from optional Flutter-inspired compositions.

#### Add sharing, store review and tracking authorization plugins

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #65](https://github.com/ifiokjr/leptoskit/pull/65)

Add independent `sharing`, `store_review` and `tracking_transparency` crates with typed binary guest clients and automatic native registration.

Sharing presents one bounded text value or HTTP(S) URL through iOS, Android, macOS or supported browsers. It does not share files or confirm delivery. Store review reports owned-context readiness and accepts explicit StoreKit or Google Play requests without claiming a prompt appeared or a review was submitted. Tracking transparency exposes iOS ATT status and explicit authorization requests without advertising identifiers or tracking collection; other platforms report unavailable.

Validate pinned Android Maven dependency metadata and emit deduplicated dependencies as Gradle input data. Validate declared Apple usage-description keys against nonempty app-owned iOS purpose strings before native preparation, without generating consent text.

Retain platform presentation ownership across cancellation where the OS cannot dismiss a flow. Extend browser and generated Android fixtures with controlled callbacks and synthetic payloads; automated tests do not open real sharing, review or tracking-consent prompts.

#### Persist shared application state across native and web

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #42](https://github.com/ifiokjr/leptoskit/pull/42)

`leptoskit_use` now supplies platform-neutral async storage hooks and optional persistent JSON storage: native application files and browser local storage implement the same contract. Async saves and reloads serialize per handle, preserve the last successful value on error, and report owner disposal instead of updating dead signals. Native async file operations run off the UI thread without requiring a specific runtime.

The `leptoskit` `storage` feature enables these adapters. Viewport and breakpoint hooks also accept fractional logical pixels, matching the native Tailwind renderer. Shared-screen tests save and reopen preferences through both native and browser controls.

#### Fix query lifecycle, cancellation, and cache identity

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #207](https://github.com/ifiokjr/leptoskit/pull/207)

Queries now leave the cache when they should and never take stale data:

- `use_queries` keeps every handle alive as its key list grows, rebinds when its queries are removed, and releases its observers on unmount, so those queries stop counting as active and are garbage collected. An observer that is dropped always unregisters from its query.
- A cancelled or superseded fetch no longer writes anything. Resetting a query before its fetch starts leaves it idle (and `use_is_fetching` drops back to zero), a response racing `cancel_queries` no longer overwrites an optimistic update, and an older response can no longer overwrite a newer one.
- Data written with `set_query_data`, `set_query_data_at`, or `update_query_data` is garbage collected after its gc time like any other unobserved query, so writes now start a timer task on the executor. A query keeps at most one cleanup timer, a stale timer can no longer evict the query that replaced it, and native hosts without a tokio runtime share one timer thread instead of starting a thread per sleep.
- Checking whether queries are active no longer subscribes the caller to every observer's `enabled` closure.

Cache identity is now explicit:

- An unnamed `QueryScope` or `InfiniteQueryScope` whose fetcher closure captures values panics on first use, rather than sharing one cache with every scope built from the same closure. Name it with `.with_name(..)`.
- Key hashes and named scope ids are stable across platforms and Rust releases (`usize` and lengths hash as 64-bit), so a server's dehydrated cache matches the browser's. `QueryClient::dehydrate` panics for an unnamed scope, whose id differs between binaries. Keys whose hashes collide are compared for equality and stay separate queries.

`InfiniteQueryScope` builders no longer change clones made earlier, and gain `with_retry_delay` and `with_data_eq`. The docs now say that hydrated data refetches in the background unless its scope has a stale time, and the optimistic update recipe cancels in-flight fetches and removes the optimistic entry when there was nothing to roll back to.

In `leptoskit_use`, `signal_debounced` and `signal_throttled` no longer notify subscribers when a write leaves the value unchanged.

#### Build and run leptos SSR and islands apps

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #31](https://github.com/ifiokjr/leptoskit/pull/31)

`app.render` picks how a web build renders: `"csr"` (the default), `"ssr"`, or `"islands"`. It replaces the `app.ssr` and `app.hydration` flags, which nothing read. In the server-rendering modes, `leptoskit build` compiles the server package with its `ssr` feature and the wasm client with its `hydrate` feature, adds `leptos/islands` to both in islands mode, and stops with a message naming the package and the fix when a feature or the leptos dependency is missing.

One package can now be both the server binary and the `cdylib` client: the client build compiles only the library, and the app package wins client detection when it has a `cdylib` target.

`leptoskit dev` and `leptoskit serve` run the app's own server binary in the server-rendering modes, with the `LEPTOS_*` environment pointing at the built site, and `dev` restarts it after each rebuild. `leptoskit_axum::static_files` serves the built site behind an app's own routes.

A new `islands` example shows a complete islands app built by leptoskit.

- **Add browser Rive playback.** Add `leptoskit_rive` and the optional `leptoskit::rive` export. Browser players use an app-supplied official runtime, accept bounded app-owned assets, reject external asset loading, disable file-triggered navigation, and release their observers and renderer on drop. Playback respects visibility and reduced motion; canvas allocations are bounded. Native widget rendering is not yet available. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #226](https://github.com/ifiokjr/leptoskit/pull/226)

#### Render routes and navigate between them

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #125](https://github.com/ifiokjr/leptoskit/pull/125)

The generated `routes.rs` now renders: `Navigation` is the app's router, and `navigation.render()` calls the current route's page inside the layouts of every directory above it. Pages and layouts receive the props they declare by name: `route` (a page's struct, or the whole `Route` for a layout), `navigation`, and a layout's `children`. Both a lowercase `page` and a `#[component] Page` are called the same way, and a parameter the router cannot pass is a generation error at its file and line.

`navigate`, `replace`, `back` and `can_go_back` change the route. Natively, earlier routes stay in memory. In the browser, `Navigation::from_browser()` (the `web` feature) starts from the page's URL, adds a history entry per navigation, and follows the back and forward buttons. `navigation.link(to, label)` builds a pressable that navigates; `Element::href` makes any pressable a link, which the web renders as an `<a href>` that opens in a new tab on a modified click while a plain click stays in the app.

#### Harden the published RPC and its generated clients

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #208](https://github.com/ifiokjr/leptoskit/pull/208)

`#[api(name = "...")]` now also sets the route, `/api/<name>`, so two same-named functions in different modules no longer share one silently; `export_if_requested` checks every start and refuses to run a server where two functions share a name or a route (`leptoskit_rpc::spec::check` runs the same check on its own). `#[api]` rejects, with errors that point at the cause, websocket protocols and the streaming and multipart encodings wherever they are named (a plain `protocol = Http<...>` is now accepted), `#[server(flatten)]` arguments, the positional `#[server]` form without an endpoint, and raw-identifier function names; when it rejects a function it keeps the function, so its error is the only one. `#[server(skip)]` arguments leave the spec, doc comments on arguments document them, wrapped `# Arguments` entries keep every line, raw argument names drop their `r#`, trailing commas in `#[server(...)]` work, and `#[server(custom = Wrapper)]` functions register under `ssr`. `DeleteUrl` functions are described as sending their arguments in the query string.

`FnError::retryable` is now true only for network failures: `ServerFnError::new` is also how a function reports "not found" or "forbidden", and every generated client follows suit. Retry backoff uses `leptoskit_query`'s timer, so it works without a Tokio runtime and in browser workers. Export failures print the IO error that caused them.

Generated clients no longer let schema, function, or argument names shadow the client's own runtime or the language's built-ins (`Client`, `Error`, `Record`, `call`, `headers`, `ctx`, and more get a numeric suffix), the Rust client no longer gives an argument named `result` the function's result type, the Go client keeps every digit of numbers in query strings and JSON bodies and compiles for string-enum results, and TypeScript argument docs can no longer close their comment early. OpenAPI no longer claims custom error types are strings. The `leptoskit` crate gains `rpc-ssr` and `rpc-axum` features that forward `leptoskit_rpc`'s server side.

#### Add a binary runtime plugin bridge

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #37](https://github.com/ifiokjr/leptoskit/pull/37)

Introduce host-granted, bounded native calls and events with interpreter imports, a cancellable Rust guest client, C/Swift/C++/JNI adapters, and independent crypto, OS-information, and clipboard plugin crates. Add `leptoskit generate app` and `leptoskit generate plugin`, native language linting, and a researched inventory of the remaining Expo, Tauri, and Electron capabilities.

This establishes the runtime infrastructure and initial APIs. It does not yet provide the full upstream catalogs, automatic native registration, or complete generated iOS/Android application hosts.

#### Pad shared screens by the host's safe-area insets

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #102](https://github.com/ifiokjr/leptoskit/pull/102)

Padding utilities accept `safe` and `safe-offset-*` values, following the `tailwindcss-safe-area` plugin: `pt-safe` pads by the top inset the host reports, and `pb-safe-offset-4` adds a spacing value to the bottom inset. Native hosts supply the insets through `NativeSurface::set_safe_area` and `StyleContext::safe_area`. `UIKitHost` forwards its controller view's `safeAreaInsets`, and the browser emits `env(safe-area-inset-*)`. Desktop windows report no insets, so a narrow desktop window no longer gets a status-bar gap meant for phones.

The gallery's phone screens use these utilities in place of fixed top and bottom padding. The Android gallery reads its window insets. Every screen now scrolls instead of clipping on short phones such as a 320×640 display.

- **Add independent accelerometer, gyroscope, magnetometer and barometer crates with shared sensor acquisition/codecs and cancellable fresh samples.** Add temporary mobile brightness, owned display wake leases and text-to-speech with voice discovery and completion/cancellation lifecycle. Hardware and browser support varies; these APIs do not promise lossless sensor recording or physical output merely because a request completes. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #53](https://github.com/ifiokjr/leptoskit/pull/53)

#### Add controlled native and web components

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #46](https://github.com/ifiokjr/leptoskit/pull/46)

Add Switch, Checkbox, Radio, Slider, LinearProgressIndicator, ActivityIndicator, PasswordInput, and activation-only Pressable. AppKit renders platform controls and the optional web adapter renders semantic DOM controls. Boolean and numeric events remain typed, control values stay application-owned, and rejected proposals restore the committed state. Validate radio groups, slider ranges and steps, progress values, and decorative button content before replacing a committed tree.

Append WASM kind and event codes without changing existing numeric codes. Extend the widget tester with boolean, radio, slider, password, and pressable interactions. Canonicalize NativeKind JSON strings while accepting legacy built-in names.

Add the shared component studio, migrate settings to actual switches, and document the component catalog with React Native and Flutter mappings and explicit platform limits.

#### Add shared layout and presentation components

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #43](https://github.com/ifiokjr/leptoskit/pull/43)

Build portable screens with Row, Column, Flex, Wrap, Grid, Padding, SizedBox, ConstrainedBox, AspectRatio, Align, Center, Expanded, Spacer, Stack, Positioned, ColoredBox, ClipRect, ClipRRect, Opacity, DefaultTextStyle, Visibility, Divider, VerticalDivider, Card, Badge, Chip, ListTile, AppBar, Scaffold, ListView, and GridView. All builders return the existing Element type and accept built-in Tailwind styles. Lists and grids are eager collections.

Correct logical flex cross-axis alignment for RTL and wrap-reverse. Resolve absolute grid children against their assigned grid area, including padding, borders, automatic edges, and alignment.

#### Ship smaller wasm, and see what you ship

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #32](https://github.com/ifiokjr/leptoskit/pull/32)

A new `[bundle]` table in `leptoskit.toml` holds everything about what a release build ships. `bundle.optimize` picks how hard the wasm client is optimized: `"fast"`, the default, compiles with your `release` profile, and `"smallest"` adds fat LTO, one codegen unit, `opt-level = "z"`, and `panic = "abort"` through a profile leptoskit defines on cargo's command line. That's 10% to 14% less wasm, for release rebuilds about three times slower. `leptoskit build --optimize` overrides it for one build, and `bundle.cargo_profile` compiles the client with a profile of your own instead. `build.wasm_opt` moves to `bundle.wasm_opt` (a configuration still setting it under `[build]` gets an error naming the new place), `wasm-opt` now runs at `-Oz`, and release wasm drops its name and producers sections.

Release web builds write brotli and gzip copies of every compressible file (`bundle.precompress`, on by default), and dev builds remove them so a stale copy is never served. `leptoskit_axum` now prefers the brotli copy when the browser accepts it.

Every release build prints the client bundle raw, gzipped, and brotli-compressed, totalled as the initial load, and `bundle.budget_kib` fails a release build whose initial load is heavier than the budget.

- **Add independent application-private SQLite and JSON settings crates with parameterized transactions, bounded operations and worker registration.** Add scoped direct asynchronous HTTP and WebSocket clients for Rust hosts and browsers, with deadlines, cancellation and bounded buffers; guest networking handles remain unimplemented. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #51](https://github.com/ifiokjr/leptoskit/pull/51)

#### Share built-in components and Tailwind across platforms

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #29](https://github.com/ifiokjr/leptoskit/pull/29)

Use View, Text, Button, Image, ScrollView, and TextInput builder components in native and web screens. Built-in Tailwind utilities resolve to native layout and visual properties or scoped CSS, including responsive and dark variants, with diagnostics for unsupported classes. The native surface retains source styles across environment changes and resolves inherited text before measurement and painting.

Correct content-box sizing, negative margins, relative and absolute insets, reversed RTL rows, and fractional minmax grid tracks. Deliver hidden and clipped states to AppKit and hit testing. Migrate the gallery and examples to the shared primitives and document CSS coverage and remaining limits.

#### Render shared screens in an interactive terminal

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #229](https://github.com/ifiokjr/leptoskit/pull/229)

Add `leptoskit_tui` and the `leptoskit` facade's `tui` feature: the same element tree and Tailwind classes every platform renders, committed into the terminal's cell grid. Layout runs on the shared 8×16-px cell so measured heights are painted rows, every built-in component draws (buttons as `[ Save ]`, switches as `[●]`, sliders, progress, a braille spinner), and colors quantize to the terminal's depth down to `NO_COLOR`. Keyboard focus, the block caret, field drafts, and scroll offsets are owned by the backend and surface through the shared `NativeEvent` model, so a terminal app is the same screens with the same handlers — the Ink model, for long-lived apps rather than short-lived CLIs.

`leptoskit new` now generates a `tui/` runner that loads the app's Wasm guest over the terminal backend, and `leptoskit build --platform tui` builds it on every operating system. The gallery gains `gallery-tui` plus a conformance test that snapshots all seven shared screens as text, and the terminal backend keeps at least 90% region coverage with fully headless sessions.

#### Draw polished terminal interfaces like opencode

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #229](https://github.com/ifiokjr/leptoskit/pull/229)

The terminal backend can now draw interfaces as detailed as the terminal apps people already use. `examples/opencode` proves it: a coding-agent interface matched cell for cell against opencode 2's terminal app (logo, prompt, a conversation with a split diff and shell output, a context sidebar, and a command palette over a dimmed screen), written as file routes and built by the leptoskit CLI for the terminal and the browser.

What changed to make that possible:

- **Styled text runs.** `Element::spans` gives a `Text` runs with their own color, background, weight, and slant (`TextSpan`), which flow and wrap as one paragraph: syntax highlighting, a bold word in a sentence, a bright key beside a dim hint. The terminal paints each run and the web renders inline `<span>`s; other backends draw the joined text in the `Text`'s own style.
- **Italic.** The `italic` and `not-italic` utilities set `NativeProps::italic`, which the terminal draws with its italic attribute and the web with `font-style`.
- **Per-edge borders.** `border-t`, `border-r`, `border-b`, `border-l`, `border-x`, and `border-y`, with an optional width, set `NativeProps::border_widths`. The terminal draws light lines for one pixel and heavy ones from two, and the web draws each edge. The native widget backends leave a per-edge border undrawn.
- **Half rows.** In the terminal, a box's height keeps half a row of precision: a panel with `pt-4 pb-2` closes with a row of `▀`, and a vertical border ends in `╹`.
- **Scrims.** A translucent background dims everything painted behind it in the terminal, and an opaque one hides the glyphs it covers. Text keeps the background painted under it.
- **The real cursor.** A focused text field's caret is the terminal's own cursor, so it blinks in the user's cursor style.
- **Keyboard shortcuts.** `Element::on_key` hears every key pressed while focus is inside the element, before the focused field acts on it, as `NativeEvent::Key(KeyPress)` with web key names. The terminal host offers each key to the app first, wasm guests receive it (ABI event kind 8), and the web renderer delivers `keydown`, keeping a handled key from the browser. `TuiApplication::dispatch` now reports the outcome.
- **Controlled focus.** `Element::focused(true)` asks for keyboard focus: the terminal moves it after each commit and the web after each render, and focus returns to the next widget still asking when a dialog closes.
- **Arbitrary breakpoints.** `min-[968px]:` and `max-[968px]:` variants, resolved natively and as media queries.
- **The viewport in guests.** A guest that exports `leptoskit_viewport(width, height)` hears the host's viewport before `leptoskit_main` and on every resize; `leptoskit::guest::viewport()` reads it.
- **Guests that link browser bindings load.** wasm-bindgen placeholder imports, such as the `console.warn` that reactive_graph (and so the file router) links, resolve to stubs that trap only if called, so routed apps run in native guests.
- **Pinned content.** `justify-content: flex-end` and `center` now overflow the start edge when their items do not fit, as CSS does, so a conversation stays pinned to its newest message on every platform.
- **The CLI builds and runs terminal apps.** `leptoskit dev --platform tui` builds the terminal app and runs it here. Client-rendered apps build without a server package, a crate with `web` and `native` features is one app for both, and a platform host may be a Cargo workspace of its own. The generated terminal host's dependencies are gated on the architecture alone.
- **No more Trunk.** The gallery's and the opencode example's browser versions build and serve with `leptoskit dev`.

#### Add widget testing and device test execution

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #28](https://github.com/ifiokjr/leptoskit/pull/28)

Add `leptoskit_testing` with semantic finders, event interactions, bounded pumping, viewport changes, and diagnostic tree dumps over the production renderer. Use it for the gallery counter and real AppKit views.

Add `leptoskit test run` for native test executables on desktop, iOS simulators, and Android devices/emulators, plus `leptoskit test web` for browser component tests. Each run retains command output and a JSON report. Devices are selected explicitly, and simulators/emulators started by a run are shut down afterward.

#### Draw filled and stroked paths with `Vector`

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #219](https://github.com/ifiokjr/leptoskit/pull/219)

`Vector(view_box)` draws SVG-style paths natively: `CAShapeLayer`s on macOS and iOS, an `android.graphics.Path` on Android, cairo on Linux, and anti-aliased GDI+ on Windows, with inline SVG on the web. Paths come from SVG path data (`M`, `L`, `H`, `V`, `C`, `S`, `Q`, `T`, `Z`, absolute or relative) through `PathData::parse`, or from generated `PathCommand`s, and each one takes a fill, a stroke with caps and joins, or both. `VectorScaling::Fit` keeps a drawing's shape, centred in its frame; `VectorScaling::Stretch` fills the frame on both axes. Stroke widths stay in layout units at any scale, so stretched outlines keep an even line.

Native apps had no way to draw a vector shape before: `Image` loads only local files there.

#### More of SVG in `Vector`, and cheaper drawings

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #222](https://github.com/ifiokjr/leptoskit/pull/222)

`Vector` now covers much more of SVG, on every platform:

- Path data accepts elliptical arcs (`A`), and `PathData` builds rectangles, rounded rectangles, circles, ellipses, lines, polylines, and polygons directly. `Affine` transforms paths exactly and parses SVG transform lists.
- Fills and strokes take `Paint::CurrentColor`, which follows the element's text color, and fills take an even-odd fill rule.
- Strokes take dashes and a dash offset, a miter limit, and `StrokeScaling::Drawing` for widths that scale with the drawing, as SVG's do by default.
- `VectorScaling::Fill` covers the frame like SVG's `slice`, and `.align(...)` places fitted and filled drawings like `xMinYMax` and its siblings.
- With the new `svg` feature, `Drawing::parse_svg` imports SVG documents: shapes, groups, transforms, styles, CSS colors, opacity, and stroke properties. Unsupported features are errors that name them.

Drawings are cheaper too. Paths and drawings are shared, immutable values, so cloning, resolving styles, and diffing an unchanged drawing cost a pointer copy or comparison instead of a walk over every point. Path data is formatted once per path for the web and the wire, guest renders reuse the wire bytes of unchanged nodes, and native backends skip redrawing unchanged drawings.

`NativeProps::drawing` is now an `Option<Drawing>` rather than a boxed one, and `VectorPath::stroke_style` returns a reference.

#### Write shared screens as markup with `view!`

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #106](https://github.com/ifiokjr/leptoskit/pull/106)

`leptoskit::prelude` now exports a `view!` macro and a `#[component]` attribute from the new `leptoskit_macros` crate. `view!` parses rstml markup, the syntax leptos uses, and expands it into the builder calls leptoskit screens already make, so markup renders on the web and on every native backend: `view! { <View class="p-4"><Text>"Count: " {count}</Text></View> }`.

`#[component]` keeps a function's signature and gives it a props builder, so `view!` can set its props by name, with leptos's `#[prop(optional)]`, `#[prop(default = …)]`, and `#[prop(into)]`. A missing or repeated prop is a compile error that names the prop.

To support them, `leptoskit_native` adds `IntoView`, `IntoChildren`, `Fragment`, and `Children`; `Element` gains `source`, `checked`, `radio_group`, `slider`, `progress`, and `running` setters; and a `Text` or `Button` now takes plain text children as its content. The gallery's screens are written with `view!`, and a snapshot test checks that they build the same element trees as before.

- **Embed wasm-bindgen 0.2.129.** The CLI now binds clients with wasm-bindgen 0.2.129, and `leptoskit new` pins the client to it. wasm-bindgen only binds wasm built against the same schema, and since 0.2.128 its schema version no longer moves with every release, so `leptoskit build` and `leptoskit doctor` now accept any locked wasm-bindgen from the embedded schema's release (0.2.128) up to the embedded 0.2.129, and name the exact `cargo update --precise` fix otherwise. Projects on 0.2.100 need that update. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #34](https://github.com/ifiokjr/leptoskit/pull/34)

#### Split lazy code into chunks that load on demand

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #35](https://github.com/ifiokjr/leptoskit/pull/35)

`bundle.split = true` runs wasm-split over the client in process before wasm-bindgen: every `#[lazy]` function, lazy route, and `#[island(lazy)]` moves into a wasm chunk that loads the first time it's needed. leptoskit links the client with `--emit-relocs`, writes the chunks, their loader, and the `__wasm_split_manifest.json` that leptos' `HydrationScripts` reads to preload chunks, binds the main module, and runs `wasm-opt -Oz` over every chunk in release. The bundle report marks chunks as lazy and leaves them out of the initial load.

A client with `#[lazy]` code and `split = false` now fails to build with the fix, instead of shipping a page whose loader import cannot resolve. A split build with `RUSTFLAGS` set must include `--emit-relocs` in it.

`leptoskit_cli` now needs Rust 1.95 or newer, for the `if let` guards wasm-split uses. In the `islands` example, splitting off one lazy island cuts the initial load from 86.8 KiB to 78.7 KiB brotli-compressed.

#### Plugin hooks for codegen, commands, native needs, serving

_Packages:_ 🟠 _leptoskit_plugin_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #194](https://github.com/ifiokjr/leptoskit/pull/194)

`LeptoskitPlugin` grows defaulted hooks, so existing plugins compile unchanged: `generate` runs before cargo on every `leptoskit build` and dev-server rebuild and records the files it wrote so the dev watcher ignores them, `run_command` answers a plugin's own `plugin:<namespace> <command>` commands, `native_integration` declares the Android permissions, Maven dependencies, and Apple usage descriptions a generated host needs, and `metadata` declares the plugin's CLI namespace and supported platforms — a build for an unsupported platform now fails naming the plugin.

`before_serve`, which existed but was never called, now runs before the `dev` and `serve` servers start.

- **Retain Android service wake callbacks across native threads, poll application services on the main looper, and join service shutdown before application replacement, recovery, or unload completes.** _Packages:_ 🟠 _leptoskit_bridge_ffi_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #168](https://github.com/ifiokjr/leptoskit/pull/168)

#### Add scoped process tools, logging and secure randomness

_Packages:_ 🟠 _leptoskit_plugin_crypto_, 🟠 _leptoskit_plugin_arguments_, 🟠 _leptoskit_plugin_shell_, 🟠 _leptoskit_plugin_logging_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #62](https://github.com/ifiokjr/leptoskit/pull/62)

Add independent launch-argument, scoped child-process and structured logging crates. Argument snapshots support typed parsing and explicit guest disclosure. Child execution fixes executable, arguments, environment and working directory, bounds output and deadlines, and owns direct-child cancellation/reaping. It requires host configuration and manual asynchronous guest integration.

Logging uses an app-directory factory, bounded JSONL rotation and explicit flush/close. Logging and crypto dispatch on Rust workers. Logging requires Rust 1.89. Extend crypto with operating-system random bytes and UUID v4 while preserving existing digest method IDs; browser randomness uses an explicit feature.

#### Add host-owned desktop menus and tray icons

_Packages:_ 🟠 _leptoskit_plugin_menu_, 🟠 _leptoskit_plugin_tray_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #66](https://github.com/ifiokjr/leptoskit/pull/66)

Add independent `menu` and `tray` crates built on muda and tray-icon, without a Tauri application. Menu definitions are bounded and validated before native allocation. macOS hosts attach an explicit application-menu lease and Windows hosts an HWND menu lease; dropping a lease restores the previous menu. Activations flow through a bounded event channel that the host installs explicitly.

The tray crate owns up to 16 icons on macOS/Windows and one KSNI icon on Linux, with validated RGBA images, optional menus, coalesced bounded events and ordered teardown. Linux deliberately avoids GTK; standalone Linux menu bars, standard roles, context menus and guest registration are not provided.

#### Add foreground native services and readable Android chrome

_Packages:_ 🟠 _leptoskit_plugin_notifications_, 🟠 _leptoskit_plugin_local_authentication_, 🟠 _leptoskit_plugin_location_, 🟠 _leptoskit_cli_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #62](https://github.com/ifiokjr/leptoskit/pull/62)

Add independent local device-owner authentication, foreground location and immediate local notification crates. Selected services register through Swift/Kotlin metadata. Permission prompts remain explicit, purpose strings remain app-owned, and cancellation/close release native work. Browser adapters expose supported location and notification operations.

Authentication does not identify an account or authorize a server request. Notification scheduling, push registration and action routing, background location, and Windows/Linux local authentication remain unimplemented. Generated Android applications use an app-owned light theme with dark system-bar icons.

#### Add owned desktop shortcuts and window placement

_Packages:_ 🟠 _leptoskit_plugin_global_shortcut_, 🟠 _leptoskit_plugin_window_state_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #62](https://github.com/ifiokjr/leptoskit/pull/62)

Add independent global-shortcut and window-state crates. Shortcuts require explicit event-loop ownership, bound registrations and event queues, and release native registrations on teardown. Window placement validates portable geometry; macOS capture/restore operates on an explicitly owned window, with persistence controlled by the application.

Wayland shortcuts and Windows/Linux native window capture/restore remain unimplemented. Extend the generated Android service fixture with logging/crypto worker assertions, scoped service checks and system-bar verification. The catalog records all eight service crates and the crypto extension with operation-level platform and registration limits.

### Fixes

#### Benchmark every pull request and fix `init()` in built apps

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #45](https://github.com/ifiokjr/leptoskit/pull/45)

The repository now carries a benchmark suite under `benchmarks/`. On every pull request the `benchmarks` workflow builds the fixture app with the base and head builds of the CLI, measures both in the same headless Chromium — the js-framework-benchmark keyed-table scenarios plus time to first render, click throughput, transfer size, and heap — hyperfines the build pipeline for both binaries, and posts a sticky comment flagging regressions against `benchmarks/policy.json`. A static comparison against plain React, React Native (react-native-web), Flutter web, vanilla JS, and cargo-leptos renders into the book's new Benchmarks chapter and is refreshed on demand with `benchmark:docs`.

Running the fixture surfaced a bug in `leptoskit build`: the in-process wasm-bindgen omitted the default module path, so the generated glue's `init()` required an explicit `.wasm` URL and the scaffold's own `await init()` failed in the browser. The glue now carries the default path like `wasm-bindgen --target web` output, and the build cache rebinds existing outputs once to pick it up.

- **Benchmark the native view, wasm, and JS layers on device.** `benchmarks/native` adds `leptoskit_bench`: one binary measuring `leptoskit_native` and `leptoskit_yoga` (build, commit, relayout, diff, Tailwind resolution, dispatch, hit testing), the wasm runtime executing a Rust-compiled guest through the real rendering ABI in wasmi, and the same guest written in JavaScript running in QuickJS as an interpreter-class baseline. It runs unchanged on the host, in the iOS simulator, and on an Android emulator through `leptoskit test run`, and emits the same report shape as the browser harness, so the pull-request comment now carries a native-layer section and the Benchmarks chapter a per-platform table. A `device comparison` workflow runs the simulator and emulator on demand. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #73](https://github.com/ifiokjr/leptoskit/pull/73)
- **Drive native application services from coalesced main-thread wake notifications on macOS and iOS.** Retain asynchronous service cleanup independently of rendering controllers, await joins before replacing or unloading a mobile guest, and preserve joined failures for repeated close callers. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #167](https://github.com/ifiokjr/leptoskit/pull/167)
- **Separate Apple controller runtime ownership from native widget rendering and mobile release presentation while preserving wake-driven teardown and replacement ordering.** _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #167](https://github.com/ifiokjr/leptoskit/pull/167)
- **Finish iOS guest detachment and scene cleanup when native teardown joins with an error.** Recover failed cold candidates, keep healthy confirmed replacements running, and preserve the original failure and teardown diagnostics. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #167](https://github.com/ifiokjr/leptoskit/pull/167)
- **Preserve the selected Rust installation in the iOS probe.** The simulator probe keeps the caller-selected Cargo, rustc and toolchain when clearing Nix build variables. Missing tools or an uninstalled simulator target now fail visibly before simulator work, and a maintained boundary control covers Rust installations outside the default Cargo home. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #166](https://github.com/ifiokjr/leptoskit/pull/166)

#### Bound the cost of every code push request

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #188](https://github.com/ifiokjr/leptoskit/pull/188)

The code push server's dynamic `check`, `download`, and `delta` routes answered from a listing of the channel, so every request read, decompressed, and hash-checked every release on it. They now answer from `index.json` and at most two more files of the static layout, as the static routes always have: a request costs the same on a channel with a thousand releases as on one with two, and a flood of update checks can no longer be turned into disk and CPU work. A release whose files are on disk but which `index.json` does not name yet is not offered, which is the guarantee the static layout already made.

Storage failures in the code push routes and in `leptoskit_axum`'s SPA fallback are a `500` with a fixed message. Filesystem paths and file contents no longer reach the client.

- **Release VoiceOver when a UIKit view stops being modal.** A view that dropped `aria-modal` kept UIKit's `accessibilityViewIsModal` set, so VoiceOver went on ignoring the rest of the screen after a dialog closed or stopped being modal. Every update now sets the flag from the current state. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #253](https://github.com/ifiokjr/leptoskit/pull/253)

#### Taps land within a frame, and benchmarks check it

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #217](https://github.com/ifiokjr/leptoskit/pull/217)

Debug native apps, including everything `leptoskit dev` runs, compiled the Wasm interpreter, layout engine, and renderer without optimizations, so a tap on the starter counter blocked the interface for more than a second on a phone. Debug builds of the native hosts now compile their dependencies optimized while the app's own crates stay unoptimized for fast rebuilds, and debug assertions stay on, so the apps still follow `leptoskit dev`. On this change's test Mac, a starter counter tap dropped from about 205 ms to well under a millisecond.

The guest runtime also leaves out a node's style and classes when they are the defaults, which the host already assumes, so every node it encodes is smaller.

The native benchmark gains tap scenarios on the starter counter screen, built with the same components and guest runtime as a new app. Every pull request compares them against its base, and a tap slower than one 60 Hz frame fails the run.

- **Block executable Pressable link targets.** Pressable links accept relative URLs, fragments, and HTTP(S), mailto, tel, and sms URLs. Executable or unknown schemes and targets containing whitespace, controls, or backslashes render as ordinary buttons, preserving their press handlers. The same policy applies to server rendering, browser updates, and directly constructed or deserialized props. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #239](https://github.com/ifiokjr/leptoskit/pull/239)
- **Keep shell descendants within invocation lifetimes.** Supervise desktop shell invocations in Unix process groups and Windows Job Objects. Terminate descendants on completion, cancellation, limits and shutdown, and retain concurrency capacity until container cleanup completes. Windows keeps the slot if any job member's termination cannot be confirmed. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #237](https://github.com/ifiokjr/leptoskit/pull/237)
- **Keep code push signing keys private on Windows.** `leptoskit codepush keygen` now creates Windows signing keys with owner-only access before writing the secret, even inside a shared directory. It refuses to write a key when the filesystem cannot enforce those permissions. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #245](https://github.com/ifiokjr/leptoskit/pull/245)
- **Bound persistent plugin storage across repeated calls.** Native filesystem storage defaults to 64 MiB and 4096 entries, with disk reconciliation and serialized reservations. SQLite enforces a host-selected page quota (64 MiB by default) and rolls back full-database failures. Secure-store writes share one encrypted record capped at 2560 encoded bytes and 64 entries across platforms. Hosts can configure budgets; quota failures return `TooLarge`, and deletion releases capacity. Legacy secure-store credentials remain readable and are migrated when rewritten. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #235](https://github.com/ifiokjr/leptoskit/pull/235)

#### Benchmark leptoskit against React Native, Lynx, and Flutter

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #252](https://github.com/ifiokjr/leptoskit/pull/252)

The arena (`benchmarks/arena`) compares leptoskit with the latest releases of React Native, Lynx, and Flutter on iOS, Android, macOS, Windows, Linux, and the web. Each framework gets one self-driving app built from a shared contract.

The scenarios cover:

- **Cold open**: launch to the ready screen.
- **Big bang**: mount, update, and clear 1,000 cards.
- **Echo**: native-call round trips at five payload sizes, on the async and sync tiers.
- **Ticker wall, autoscroll, and busy hands**: frame scenarios.
- **Size tiers**: the same app at 1, 12, and 120 screens.

Frames are counted from screen recordings rather than from each framework's own counters. Every app shows a phase-coloured beacon, and the recording's deduplicated frames give presented frames per second and Apple's hitch-time ratio.

The tasks are:

- `benchmark:arena` runs every reachable target.
- `benchmark:arena:update` first moves every framework to its latest stable release, resolved from npm, Maven Central, CocoaPods, and Flutter's release feed.
- `benchmark:arena:docs` regenerates the comparison tables.

The new section in the Benchmarks chapter adds a table comparing the four frameworks' architectures: how updates and native calls cross to the screen, threading, 3D support, and over-the-air updates. An on-demand `arena` workflow measures Windows and Linux.

#### Fix release Android apps crashing at launch

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #220](https://github.com/ifiokjr/leptoskit/pull/220)

Release APKs built with `leptoskit native build android --release` crashed on launch with `NoSuchMethodError` for `NativeServiceOwner.wake()`: R8 renamed the Kotlin callbacks the native library looks up by name. The CLI now writes keep rules for exactly those members to `.leptoskit/android/proguard-rules.pro` on every Android build, so they always match the native library they guard, and new projects pass that file to R8.

Apps created before this release need one line in `android/app/build.gradle.kts`. Add the generated file to the release `proguardFiles` list, before your own rules:

```kotlin
proguardFiles(
    getDefaultProguardFile("proguard-android-optimize.txt"),
    "../../.leptoskit/android/proguard-rules.pro",
    "proguard-rules.pro",
)
```

Release builds warn until the line is there. Your own `proguard-rules.pro` can drop its `-keep class dev.leptoskit.bridge.** { *; }` line, which the generated file now carries.

- **Support Android package visibility queries that pair an intent action with a URI scheme.** Preserve app-owned manifest declarations and avoid duplicate queries. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #186](https://github.com/ifiokjr/leptoskit/pull/186)
- **Report scroll events from the AppKit backend.** `leptoskit_appkit` scroll views now deliver `NativeEvent::Scroll` with the content offset whenever their content moves, as the iOS, Android, Linux, and Windows backends already do, so a `ScrollView`'s `.on(NativeEventSlot::Scroll, …)` handler runs on macOS and a `VirtualList` follows scrolling there. A deleted scroll view stops reporting. The generated macOS app still draws through its own renderer and does not report scroll events yet. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #202](https://github.com/ifiokjr/leptoskit/pull/202)
- **See through Linux accessibles destroyed mid-observation.** Native waits on Linux now treat the AT-SPI driver's `'NoneType' object is not iterable` error as an element that vanished while being read, the same way they already treated `stale element reference`. selenium-webdriver-at-spi raises it when an accessible is destroyed during its tree walk, as a portal dialog's are while the dialog closes, so `expect_absent` and `expect_visible` keep polling until their deadline instead of failing. Actions such as `tap` still fail on it. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #140](https://github.com/ifiokjr/leptoskit/pull/140)
- **Bound code push origin work.** Code push downloads now stream in 64 KiB chunks with Range and HEAD support instead of buffering entire release and delta files. Update checks use container hashes saved at ingest or import without reading release or delta bodies. Metadata work runs off async workers, and each router rejects excess requests with 503 while 16 responses are active, including downloads still in progress. Older stores without saved hashes continue to offer full downloads through the dynamic routes. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #243](https://github.com/ifiokjr/leptoskit/pull/243)
- **Swap iOS code push guests atomically.** While an update was being confirmed, the iOS host mounted the new guest on top of the running one, and both stayed attached. VoiceOver and UI tests could reach controls from both guests at once. The host now does what the Android host already did: it renders the candidate underneath, invisible and hidden from accessibility, while the confirmed guest keeps the screen and stays interactive. On confirmation it swaps the two in one main-thread turn. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #145](https://github.com/ifiokjr/leptoskit/pull/145)

#### Harden native code push against hostile and failing states

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #82](https://github.com/ifiokjr/leptoskit/pull/82)

A 404 from a static registry now means "no release yet" on mobile: `HttpTransport` returns `CodePushError::NotFound` for it, so a channel with no `index.json` reports up to date instead of failing. Requests get 15 seconds to connect and 30 seconds for a response. A whole download gets 10 minutes, so large bundles arrive over slow links.

`CodePushClient::with_max_container_bytes` caps what a delta may rebuild, and `InstalledBundles` sets it to 32 MiB, so a hostile delta can't allocate more than a full download would. `apply_delta_within` takes the cap directly.

A new store build supersedes every earlier download, even when the app version is unchanged. Only the working release and a staged one stay on disk. Interrupted writes no longer leave temporary files behind.

The generated iOS and Android hosts run the store-shipped guest when the update store can't open or launch, instead of showing an empty screen. A guest that builds its tree after an asynchronous step is no longer rejected for rendering nothing on its first commit.

- **Bound containers and total expansion to 32 MiB, manifests to 1 MiB, and bundles to 4096 files.** Reject over-limit container writes and release creation before changing existing files or the release store. S3 reads now enforce separate response limits for indexes (2 MiB), signatures (4 KiB), and releases or deltas (32 MiB), including responses without a content length. Omit over-limit optional deltas from publication so interrupted publishes can resume and devices download the full release. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #247](https://github.com/ifiokjr/leptoskit/pull/247)

#### Ship the mobile guest once and launch it with one read

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #101](https://github.com/ifiokjr/leptoskit/pull/101)

Generated iOS and Android apps now bundle the guest only inside its store container, `app.lkb`, instead of shipping it a second time as `app.wasm`. With code push off, or if the update store fails, the host reads the Wasm through the new `lk_bundle_entry` FFI call, which wraps `installed::entry_module` and checks every file hash. Mobile assets now live in `.leptoskit/mobile`, and Android's Gradle template points there. Desktop hosts keep `.leptoskit/assets/app.wasm`.

`InstalledBundles::launch` now reads, verifies, and decompresses only the release that runs, instead of every stored release on each launch. A release damaged since `open` is skipped: a damaged candidate is rejected, and a damaged confirmed release falls back to the store baseline.

- **Update constant-time utilities.** Update the locked ctutils dependency to 0.4.4, which fixes RUSTSEC-2026-0385 in mask generation and 128-bit comparisons on 32-bit targets. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #164](https://github.com/ifiokjr/leptoskit/pull/164)
- **Update constant-time dependency to patched ctutils.** Refresh the Cargo lockfile to ctutils 0.4.4, fixing RUSTSEC-2026-0385 in constant-time mask generation and u128 comparisons. Existing public APIs are unchanged. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #165](https://github.com/ifiokjr/leptoskit/pull/165)
- **Reject links to local addresses in docs.** A built page that links to `http://localhost:3000`, `http://127.0.0.1:3000`, or another loopback address sends every reader to their own machine, where nothing is listening. GitHub-flavoured autolinks made this easy to do by accident: a bare dev-server URL in prose, or one wrapped in angle brackets, became a link. `Site::build` now reports these links with their file, line, and column, and asks for inline code instead. Addresses inside code spans and code blocks are never links, so they are unaffected. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #173](https://github.com/ifiokjr/leptoskit/pull/173)
- **Rebuild the served docs when the watcher drops events.** Under load, macOS FSEvents and Linux inotify drop events and report only a rescan that names no file. `Site::serve` ignored those, so an edit could fail to rebuild the site or reload open pages. Rescans and watcher errors now count as changes, and the server also compares the book's file sizes and modification times every second, so an edit no watcher reported still rebuilds. The output directory is created before watching so its writes are never mistaken for edits, which could otherwise rebuild in a loop after a failed first build. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #154](https://github.com/ifiokjr/leptoskit/pull/154)
- **Keep documentation sources inside the book.** Reject unsafe chapter paths in `SUMMARY.md`, including absolute paths, parent traversal, and Windows path prefixes or separators. Refuse summaries and chapters whose symlinks resolve outside the documentation source directory before rendering or replacing any output. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #234](https://github.com/ifiokjr/leptoskit/pull/234)
- **Pin every checked-in `leptoskit.toml` to `schema_version = 0` and the public v0 JSON Schema so examples, benchmarks, and CLI fixtures have editor autocomplete and diagnostics.** Preserve the required `#:schema` directive through TOML formatting and use the same syntax in documentation examples. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #183](https://github.com/ifiokjr/leptoskit/pull/183)
- **Wrap flex lines without a gap before their first item.** A wrapping flex container counted the gap before an item that moved to a new line as part of that line, so lines filled up one gap early and could wrap an item sooner than CSS does. A line now holds only the gaps between its own items. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #203](https://github.com/ifiokjr/leptoskit/pull/203)
- **Isolate concurrent function specification exports.** Concurrent CLI exports sharing a Cargo target now use separate temporary output directories. A server that writes no specification reliably reports the missing export, and timeout cleanup joins the server before releasing its output directory. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #169](https://github.com/ifiokjr/leptoskit/pull/169)

#### Re-export JNI bindings from the Android backend

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #78](https://github.com/ifiokjr/leptoskit/pull/78)

`leptoskit_android` (and `leptoskit::android`) now re-exports `jni`, so an Activity's JNI entrypoints can construct `AndroidBackend` with the same `jni` version the backend uses, without declaring the dependency themselves.

The gallery example renders its shared screens with every native backend: AppKit, GTK, Win32, UIKit, and Android Views. CI snapshots every screen on each desktop OS and fails unless the iOS app and the Android instrumentation report every screen as verified.

- **Escape Go RPC struct tags.** Encode schema property and parameter names before writing Go struct tags, preventing consumed RPC specifications from injecting Go source through backticks, quotes, or control characters. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #241](https://github.com/ifiokjr/leptoskit/pull/241)
- **Keep GTK text input placeholders inside their frames.** GTK text inputs no longer clip their text or placeholder vertically. Yoga sizes an input from its text, but the GTK theme gave every `GtkEntry` a 34px minimum height and the frame placed that taller entry from its top edge, so the centered placeholder was pushed down and cut off at the bottom of the frame, as in the gallery's settings search field. Entries now drop the theme's minimum height, and a native control taller than its frame is centered on it, so clipping removes chrome rather than text. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #99](https://github.com/ifiokjr/leptoskit/pull/99)
- **Hooks run in native apps.** `leptoskit::hooks` and Leptos signals work inside a native app's Wasm runtime on iOS, Android, macOS, Windows, and Linux, not only in the browser. The runtime calls a mounted view again after every event, so screens read their signals' current values; call `leptoskit::guest::render()` after a change that happens outside an event, such as a plugin reply. A compiled guest that uses hooks now runs through the interpreter in CI, so a dependency update that would stop such a guest from loading fails there first. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #216](https://github.com/ifiokjr/leptoskit/pull/216)
- **Pin generated CLI output with insta snapshot tests.** The CLI's integration tests now capture everything it generates — scaffolded projects file by file, help text for every subcommand, error wording, and code push release artifacts including packed manifests — in committed insta snapshots, so any change to generated output arrives as a reviewable `.snap` diff instead of passing silently. Test projects moved into a `tests/fixtures` directory shared by the test suites, and the insta dependency gained the `filters` and `redactions` features for the `[ROOT]` path stand-ins and the manifest timestamp redaction. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #24](https://github.com/ifiokjr/leptoskit/pull/24)

#### Harden translations: names, variants, scripts, quoting

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #206](https://github.com/ifiokjr/leptoskit/pull/206)

`leptoskit l10n` now rejects, with the file and line, ARB files that used to generate a module that misbehaved or did not compile: a key that would become one of the `L10n` handle's own methods (`locale`, `set_locale`, `messages`, `clone`, `into`, and the rest), which `l10n.locale()` would call instead of the message; a key or placeholder named `gen`, reserved from Rust 2024; an `=N` form larger than its placeholder's type holds; and quoted text that reads as a placeholder, such as `'{name}'`, which a Flutter project without `use-escaping` means as the placeholder. A key that is not an identifier says why instead of calling it a keyword, and problems are listed by file and then by line. Outside a plural, `'#'` now stays as written, as in ICU.

Locales with variants, such as `ca-ES-valencia` next to `ca-ES`, each get their own messages, and a placeholder can no longer shadow a locale's plural rules. Forms with the same text share one `match` arm or `if` branch, a select or plural with only `other` is its text, and a placeholder a translation leaves unread is named `_name`, so the generated module passes clippy's pedantic lints. Message text holding backticks shows as code in the generated documentation. `leptoskit l10n --check` also stays green after rustfmt rewraps the module's documentation or splits its long strings.

Locale negotiation fills in the script CLDR expects for languages written in more than one, so `zh-TW` picks `zh-Hant` over `zh-Hans`, and the docs list where it chooses differently from Flutter. `accept_language` leaves out entries whose quality value RFC 9110 does not allow (`q=NaN`, `q=5`), and `browser_locales` is exported with or without the `web` feature, returning an empty list wherever there is no browser to ask.

- **Render canonical widget names in generated macOS hosts.** The AppKit host accepts the canonical widget names serialized by the Rust renderer and retains legacy aliases, restoring the generated application's controls after the shared component update. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #50](https://github.com/ifiokjr/leptoskit/pull/50)
- **Cache validated Wasm export handles and measure initialized Rust/Wasm/C-ABI plugin round trips, including owned replies and allocation accounting.** The benchmark excludes OS work and UI scheduling and does not establish performance superiority over other frameworks. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #49](https://github.com/ifiokjr/leptoskit/pull/49)

#### Paint the whole screen and draw every shape and label

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #227](https://github.com/ifiokjr/leptoskit/pull/227)

A dark screen in a generated iOS or Android app stopped at the safe area: the status bar, the home indicator, and Android's navigation bar stayed white around it. The generated apps now paint those strips with the root view's opaque background, the way a browser paints the page's background across the whole window, and switch the system bar icons to light on a dark background and dark on a light one. A root without an opaque background keeps the system's background, as before.

The code-push update bar keeps its light background whatever the guest paints.

Pill shapes drew nothing in generated iOS apps. `rounded-full` asks for a 9999-point radius, and a clipping layer whose radius exceeds half its size draws nothing, so pill buttons, badges, and progress tracks vanished. The radius is now scaled to fit each frame, as browsers and the UIKit backend crate already do.

Generated Android apps truncated each widget's frame to whole pixels, dropping up to a pixel. With measured text, a label sized exactly to its words wrapped its last word out of view. Frames now snap their edges outward instead.

- **Prune native hit testing with cached subtree bounds.** `NativeSurface::hit_test` skips subtrees that cannot contain the point. Each layout commit caches, per node, the union of its frame and its unclipped descendants' frames, so a query reads that dense cache instead of every node's props and layout. Results are unchanged: clipping, rounded clipping, hidden subtrees, stopped activity indicators, and decorative `Pressable` content all hit exactly as before. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #77](https://github.com/ifiokjr/leptoskit/pull/77)
- **Preserve explicit line breaks in native text layout.** Measure every newline-delimited line independently, retaining consecutive and trailing empty lines. Native layouts reserve their combined height while preserving word wrapping, font scaling and the widest line's width. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #172](https://github.com/ifiokjr/leptoskit/pull/172)
- **Exercise registered plugins with a generated Android instrumentation suite, browser lifecycle checks and an interpreted binary-reply fixture.** Document operation-level platform coverage, registration modes, bridge limits and remaining Expo/Tauri/Electron scope. Private validation examples are excluded from release package registration. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #55](https://github.com/ifiokjr/leptoskit/pull/55)

#### Centre text and buttons on iOS and Android

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #221](https://github.com/ifiokjr/leptoskit/pull/221)

Screens laid out with `items-center` now centre their content on the screen in the apps `leptoskit new` generates. Three things kept them off centre:

- The generated iOS and Android apps measured text with a fixed-width approximation, so a label's box rarely matched the text the platform drew in it. They now measure with the platform's own fonts: Core Text on iOS, and the typeface and scaled size Android's text views use.
- A screen's root `View` or `ScrollView` shrank to its content instead of filling the window's width as its `<div>` does in a browser. It now fills the width, less its margins and within its max width; text and control roots keep their intrinsic width.
- Android buttons left-aligned their label and set it in capitals. They now centre it unless the element sets `text-left` or `text-right`, and show the text as written, like iOS and the web.

Apps generated earlier pick up the measurement and alignment changes from the CLI. Their release builds also need `-keepclassmembers class dev.leptoskit.mobile.NativeTextMeasurer { float[] measure(byte[], float, int, float, boolean); }` in `android/app/proguard-rules.pro`.

- **Time out native waits whose deadline expires mid-response.** A native UI wait that runs out while reading a driver's response body now fails with the wait's `Error::Timeout`. Previously, reqwest reported that read's timeout as an I/O error, so a slow driver on a loaded host failed the wait with `request or response body error` instead of naming the condition that never appeared. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #123](https://github.com/ifiokjr/leptoskit/pull/123)
- **Preserve generated host compatibility with widget wire names.** Generated UIKit and Android hosts accept the canonical widget names serialized by the Rust renderer, while retaining legacy names. This restores initial interface rendering after the shared component serialization update. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #44](https://github.com/ifiokjr/leptoskit/pull/44)
- **Release every platform backend with the rest of leptoskit.** `leptoskit_uikit`, `leptoskit_android`, `leptoskit_gtk`, `leptoskit_windows`, and `leptoskit_plugin_router` now release alongside the other leptoskit crates. Before, a release raised their versions but left the versions the other crates require of them where they were, so the first minor release would have published crates that could not resolve each other. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #274](https://github.com/ifiokjr/leptoskit/pull/274)

#### Route every URL and every file name safely

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #205](https://github.com/ifiokjr/leptoskit/pull/205)

`Route::parse` now reads URLs the way a browser does, so it never fails: a stray `%` stays as it is, escapes that aren't UTF-8 become U+FFFD, and `.` and `..` segments are resolved. `Navigation::from_browser()` mounts on any URL, and `ParseError` and `UrlError` are gone; `parse` and `from_browser` return `NoMatch`. Only path values decide whether a route matches: a query value that doesn't parse is ignored, so `/notes/42?highlight=x` is note 42, and a path parameter never reads from the query, so `/about?lang=fr` has no language.

An href never holds an empty, `.` or `..` segment and never starts with `//`: an optional segment holding one is left out, and a required segment or catch-all value holding one ends the path, so an empty value can no longer produce a link to another site.

The generator turns every file name into valid Rust, escaping modules and variants the same way (`404.rs` is `_404`, `self.rs` is `self_` and `Self_`, `gen.rs` is `gen_`), and reports a problem it cannot fix instead of panicking. A route with optional segments is ordered once for each combination of them present and absent, so `/about` reaches `[[lang]]/about.rs` before `[section].rs`, and two routes that match the same URLs through their optional segments are reported. A `_layout.rs` without a `layout` is an error, private helpers named like exports are ignored, `home.ios.rs` and the other `platform_module!` suffixes are recognised as platform overrides, and route struct fields are classified exactly as `#[derive(Route)]` reads them.

- **Encode untrusted RPC metadata in generated clients.** RPC client generation now encodes titles and versions in source comments, Python module strings, and package manifests. CLI help text prefixes every imported documentation line as a comment. Untrusted specification metadata can no longer add Cargo dependencies or inject generated commands through these fields. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #248](https://github.com/ifiokjr/leptoskit/pull/248)

#### Leave room for native chrome around unstyled text inputs

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #124](https://github.com/ifiokjr/leptoskit/pull/124)

An unstyled `TextInput` or `PasswordInput` with `h-auto` was exactly as tall as its text, so its frame clipped the top and bottom of the native text-field border, such as GTK's entry border and AppKit's rounded bezel. Text inputs that keep the native chrome now measure 12 layout units taller than their text, which makes an unstyled field 28 tall at the default font size, the same height `PasswordInput` already defaults to. Inputs with their own background, border, or corner radius draw no native chrome and still measure from their text and padding, and an explicit height class such as `h-5` still wins.

`NativeProps::is_custom_drawn` reports whether props replace a control's native chrome, the rule the GTK and AppKit backends already applied.

- **Bound guest rendering allocations.** Reject Wasm guests that exceed persistent node and property quotas, expanded accessibility text limits, or per-entry rendering work. Bound cumulative native widget slots, release deleted properties, and cap undrained mutation batches and their JSON output. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #244](https://github.com/ifiokjr/leptoskit/pull/244)
- **Bound native Wasm memory and tables.** Every native Wasm host now limits its guest to one 64 MiB linear memory and two tables with at most 65,536 elements each. The limits apply during loading and every later callback, including unexported resources and the module's start function. Oversized allocations and growth beyond these budgets return `WasmError::ResourceLimitExceeded`. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #238](https://github.com/ifiokjr/leptoskit/pull/238)
- **Scaffold web projects against a local SDK.** `leptoskit new --sdk-path <repo>` now works for web projects as well as native ones: the client depends on the repository's `crates/leptoskit` by path, so a scaffold can use features that are not released yet. Previously the flag was rejected for web projects, and a fresh web scaffold could only build against the published release. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #81](https://github.com/ifiokjr/leptoskit/pull/81)
- **Measure macOS button titles with their native control geometry and display backing grid.** Keep custom button chrome consistent between layout and rendering. Preserve existing custom text measurers, explicit caller constraints and native font sizes. _Packages:_ 🟢 _leptoskit_native_, 🟢 _leptoskit_appkit_, 🟢 _leptoskit_bridge_ffi_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #180](https://github.com/ifiokjr/leptoskit/pull/180)
- **Expose `role="heading"` to `VoiceOver` as `AXHeading` without linking `NSAccessibilityHeadingRole`, which only exists from macOS 26, so macOS apps keep launching on macOS 14 through 25.** _Packages:_ 🟢 _leptoskit_appkit_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #185](https://github.com/ifiokjr/leptoskit/pull/185)
- **Show iOS startup errors before the first layout.** _Packages:_ 🟢 _leptoskit_bridge_ffi_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #228](https://github.com/ifiokjr/leptoskit/pull/228)
- **Expose labelled iOS text as an accessibility value.** Keep visible UILabel content readable alongside a custom accessibility label. Refresh and clear the value when props change, explicitly restore ordinary text labels after removing a custom label, and isolate UIKit style mutation to the main actor. _Packages:_ 🟢 _leptoskit_bridge_ffi_, 🟢 _leptoskit_cli_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #232](https://github.com/ifiokjr/leptoskit/pull/232)
- **Remove Android Button's native padding before applying shared layout frames so compact buttons paint their labels.** _Packages:_ 🟢 _leptoskit_bridge_ffi_, 🟢 _leptoskit_cli_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #172](https://github.com/ifiokjr/leptoskit/pull/172)
- **Separate Android service scheduling, native request jobs, widget rendering, and widget properties from application lifecycle control while preserving native callback and shutdown ordering.** _Packages:_ 🟢 _leptoskit_bridge_ffi_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #168](https://github.com/ifiokjr/leptoskit/pull/168)
- **Complete joined guest retirement after cleanup errors, preserve confirmed replacements, and retain recovery diagnostics through fallback.** Guard delayed recovery errors against stale application generations. _Packages:_ 🟢 _leptoskit_bridge_ffi_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #168](https://github.com/ifiokjr/leptoskit/pull/168)
- **Prevent overlapping owners from corrupting installed updates.** Hold an exclusive file lock for the lifetime of each installed-update store. Overlapping opens, including during iOS controller replacement, return a diagnostic before reading or changing selector state. Dropping or destroying the previous owner releases the store for a retry. _Packages:_ 🟢 _leptoskit_bridge_ffi_, 🟢 _leptoskit_codepush_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #242](https://github.com/ifiokjr/leptoskit/pull/242)
- **Use the existing AppKit font measurer for macOS application layout so native multiline Text receives the space its unchanged native renderer requires.** _Packages:_ 🟢 _leptoskit_bridge_ffi_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #172](https://github.com/ifiokjr/leptoskit/pull/172)
- **Keep native Wasm guests optimized in debug apps.** Compile interpreted guests with the configured release profile while retaining native host debugging and the existing bundle size limit. _Packages:_ 🟢 _leptoskit_cli_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #230](https://github.com/ifiokjr/leptoskit/pull/230)
- **Allow one runtime package to install its Rust application service alongside a distinct native plugin on macOS, iOS, and Android.** Generated hosts retain both registrations and their native sources, imports, and context while preserving the one-service limit and duplicate capability rejection. _Packages:_ 🟢 _leptoskit_cli_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #181](https://github.com/ifiokjr/leptoskit/pull/181)
