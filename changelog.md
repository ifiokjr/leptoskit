# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.1](https://github.com/ifiokjr/leptoskit/releases/tag/v0.1.1) (2026-10-24)

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

#### Identify interpreter and transport owners

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #159](https://github.com/ifiokjr/leptoskit/pull/159)

Bridge and WasmHost expose opaque cloneable identity handles that remain stable across moves and cannot collide while an earlier handle remains alive. Services can bind work to both owners and reject delivery after replacement or transport transplantation. Handles retain no guest or interpreter state.

App::with_host lends the application's actual interpreter to a synchronous Rust closure. Native I/O can retain owned work while the application remains usable; interpreter references and futures borrowing them cannot escape the closure.

- **Register application-owned asynchronous native services.** Native runtime metadata accepts `rust-service-factory` for an inert `ServiceInstaller`. Generated macOS, iOS and Android Rust registrants defer the service until platform activation owns wake notifications and joined cleanup. Ambiguous plugin/service constructors and multiple selected service registrations fail before generation. Existing plugin and worker registration retain their behavior. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #169](https://github.com/ifiokjr/leptoskit/pull/169)

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

#### Split lazy web code by default

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #155](https://github.com/ifiokjr/leptoskit/pull/155)

Lazy functions, routes, and islands now build into on-demand wasm chunks without setting `bundle.split`. Clients without lazy code skip splitting and keep their existing binding cache. Explicit `split = false` remains available and rejects lazy imports before writing an unusable client. Environment rustflags only require relocations when the client actually contains lazy code.

Standalone clients discard linker metadata before optimization so Binaryen can produce the same small bundles as an eager build. Application custom sections are preserved, and existing binding caches are refreshed.

The native CodePush investigation includes a reproducible wasmi probe and a proposed shared API; native chunk loading is not yet a supported application feature.

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

#### Route apps by their files

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #105](https://github.com/ifiokjr/leptoskit/pull/105)

Put one file per screen in `src/routes/`, add a `[router]` table to `leptoskit.toml`, and leptoskit writes `src/routes.rs`: a module for every route file and one `Route` enum that parses URLs and writes hrefs. The file names follow Expo Router: `index.rs`, `[note].rs`, `[[lang]]/`, `[...topic].rs`, `[[...topic]].rs`, `(group)/`, `_layout.rs`, `_private.rs`, `+not-found.rs` and `+html.rs`. A route with parameters declares a `#[derive(Route)]` struct whose fields come from the path when they are named after a segment and from the query string otherwise, so a link to a missing route, or one without its parameters, does not compile. Pages are a lowercase `page` function or a capitalised `#[component] Page`.

`leptoskit routes` writes the module and lists the routes, and `--check` fails in CI when the committed module is out of date. `dev`, `build` and `native` regenerate it before compiling, writing only when the routes changed. The generator reports every problem with its file and line, and a route struct that drifts from its file name is a compile error even before the generator runs again. The new crates are `leptoskit_router`, `leptoskit_router_macros` and `leptoskit_router_codegen`; `examples/file_routes` is a complete app.

- **Expose host-assigned guest plugin and call identities for application-owned event routing.** Call identities remain available until the response future consumes its result or cancels its outstanding call. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #166](https://github.com/ifiokjr/leptoskit/pull/166)

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

#### Add `leptoskit_docs`, Markdown with leptos components

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #136](https://github.com/ifiokjr/leptoskit/pull/136)

A new crate for writing documentation the way MDX does, with leptos in place of React: a capitalised tag in a Markdown file, such as `<Callout kind="tip">`, renders a leptos component registered by name, and the Markdown between its tags becomes the component's children. Lowercase tags are plain HTML, and GitHub alerts (`> [!TIP]`) render as callouts.

`Site` builds a whole book, laid out by an mdBook-style `SUMMARY.md`, into a static site: chapter navigation, an on-page table of contents, search, build-time syntax highlighting with tree-sitter grammars, light and dark themes, and copy buttons. Every link between pages and headings is checked, so a broken link fails the build instead of shipping. `Site::serve` rebuilds on every edit and reloads open pages.

#### Format Rust and view!

markup with `leptoskit format`

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #108](https://github.com/ifiokjr/leptoskit/pull/108)

A new `leptoskit_fmt` crate, built into the CLI as `leptoskit format` (and `lk format`), replaces leptosfmt. It formats Rust with rustfmt — nightly by default, honouring the project's `rustfmt.toml` — and lays out the rstml markup inside `view!` macros (`view`, `leptos::view`, and `leptoskit::view` by default) at the same width and indentation, with every Rust expression inside the markup formatted by rustfmt at the column it sits in. Formatting is idempotent, keeps every comment, copies text and literals byte for byte, and leaves markup that does not parse exactly as written with a warning.

`--check` prints a diff and fails when anything would change, `--stdin` formats for editors, and whole-workspace runs format files in parallel and skip files a previous run left formatted. Settings live in a new `[format]` table in `leptoskit.toml`.

- **Add cross-platform reactive utilities.** Add `leptoskit_use` and the optional `leptoskit::hooks` re-export. State, derived values, watchers, timer filters, typed storage, and viewport breakpoints use Leptos signals without requiring a DOM or executor. Host clocks, viewport signals, and storage adapters make environmental dependencies explicit. CI enforces 100% line coverage for the new crate and checks browser WASM and WASI builds. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #27](https://github.com/ifiokjr/leptoskit/pull/27)
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

#### Build and run leptos SSR and islands apps

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #31](https://github.com/ifiokjr/leptoskit/pull/31)

`app.render` picks how a web build renders: `"csr"` (the default), `"ssr"`, or `"islands"`. It replaces the `app.ssr` and `app.hydration` flags, which nothing read. In the server-rendering modes, `leptoskit build` compiles the server package with its `ssr` feature and the wasm client with its `hydrate` feature, adds `leptos/islands` to both in islands mode, and stops with a message naming the package and the fix when a feature or the leptos dependency is missing.

One package can now be both the server binary and the `cdylib` client: the client build compiles only the library, and the app package wins client detection when it has a `cdylib` target.

`leptoskit dev` and `leptoskit serve` run the app's own server binary in the server-rendering modes, with the `LEPTOS_*` environment pointing at the built site, and `dev` restarts it after each rebuild. `leptoskit_axum::static_files` serves the built site behind an app's own routes.

A new `islands` example shows a complete islands app built by leptoskit.

#### Render routes and navigate between them

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #125](https://github.com/ifiokjr/leptoskit/pull/125)

The generated `routes.rs` now renders: `Navigation` is the app's router, and `navigation.render()` calls the current route's page inside the layouts of every directory above it. Pages and layouts receive the props they declare by name: `route` (a page's struct, or the whole `Route` for a layout), `navigation`, and a layout's `children`. Both a lowercase `page` and a `#[component] Page` are called the same way, and a parameter the router cannot pass is a generation error at its file and line.

`navigate`, `replace`, `back` and `can_go_back` change the route. Natively, earlier routes stay in memory. In the browser, `Navigation::from_browser()` (the `web` feature) starts from the page's URL, adds a history entry per navigation, and follows the back and forward buttons. `navigation.link(to, label)` builds a pressable that navigates; `Element::href` makes any pressable a link, which the web renders as an `<a href>` that opens in a new tab on a modified click while a plain click stays in the app.

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

#### Add widget testing and device test execution

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #28](https://github.com/ifiokjr/leptoskit/pull/28)

Add `leptoskit_testing` with semantic finders, event interactions, bounded pumping, viewport changes, and diagnostic tree dumps over the production renderer. Use it for the gallery counter and real AppKit views.

Add `leptoskit test run` for native test executables on desktop, iOS simulators, and Android devices/emulators, plus `leptoskit test web` for browser component tests. Each run retains command output and a JSON report. Devices are selected explicitly, and simulators/emulators started by a run are shut down afterward.

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
- **See through Linux accessibles destroyed mid-observation.** Native waits on Linux now treat the AT-SPI driver's `'NoneType' object is not iterable` error as an element that vanished while being read, the same way they already treated `stale element reference`. selenium-webdriver-at-spi raises it when an accessible is destroyed during its tree walk, as a portal dialog's are while the dialog closes, so `expect_absent` and `expect_visible` keep polling until their deadline instead of failing. Actions such as `tap` still fail on it. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #140](https://github.com/ifiokjr/leptoskit/pull/140)
- **Swap iOS code push guests atomically.** While an update was being confirmed, the iOS host mounted the new guest on top of the running one, and both stayed attached. VoiceOver and UI tests could reach controls from both guests at once. The host now does what the Android host already did: it renders the candidate underneath, invisible and hidden from accessibility, while the confirmed guest keeps the screen and stays interactive. On confirmation it swaps the two in one main-thread turn. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #145](https://github.com/ifiokjr/leptoskit/pull/145)

#### Harden native code push against hostile and failing states

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #82](https://github.com/ifiokjr/leptoskit/pull/82)

A 404 from a static registry now means "no release yet" on mobile: `HttpTransport` returns `CodePushError::NotFound` for it, so a channel with no `index.json` reports up to date instead of failing. Requests get 15 seconds to connect and 30 seconds for a response. A whole download gets 10 minutes, so large bundles arrive over slow links.

`CodePushClient::with_max_container_bytes` caps what a delta may rebuild, and `InstalledBundles` sets it to 32 MiB, so a hostile delta can't allocate more than a full download would. `apply_delta_within` takes the cap directly.

A new store build supersedes every earlier download, even when the app version is unchanged. Only the working release and a staged one stay on disk. Interrupted writes no longer leave temporary files behind.

The generated iOS and Android hosts run the store-shipped guest when the update store can't open or launch, instead of showing an empty screen. A guest that builds its tree after an asynchronous step is no longer rejected for rendering nothing on its first commit.

#### Ship the mobile guest once and launch it with one read

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #101](https://github.com/ifiokjr/leptoskit/pull/101)

Generated iOS and Android apps now bundle the guest only inside its store container, `app.lkb`, instead of shipping it a second time as `app.wasm`. With code push off, or if the update store fails, the host reads the Wasm through the new `lk_bundle_entry` FFI call, which wraps `installed::entry_module` and checks every file hash. Mobile assets now live in `.leptoskit/mobile`, and Android's Gradle template points there. Desktop hosts keep `.leptoskit/assets/app.wasm`.

`InstalledBundles::launch` now reads, verifies, and decompresses only the release that runs, instead of every stored release on each launch. A release damaged since `open` is skipped: a damaged candidate is rejected, and a damaged confirmed release falls back to the store baseline.

- **Update constant-time utilities.** Update the locked ctutils dependency to 0.4.4, which fixes RUSTSEC-2026-0385 in mask generation and 128-bit comparisons on 32-bit targets. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #164](https://github.com/ifiokjr/leptoskit/pull/164)
- **Update constant-time dependency to patched ctutils.** Refresh the Cargo lockfile to ctutils 0.4.4, fixing RUSTSEC-2026-0385 in constant-time mask generation and u128 comparisons. Existing public APIs are unchanged. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #165](https://github.com/ifiokjr/leptoskit/pull/165)
- **Reject links to local addresses in docs.** A built page that links to `http://localhost:3000`, `http://127.0.0.1:3000`, or another loopback address sends every reader to their own machine, where nothing is listening. GitHub-flavoured autolinks made this easy to do by accident: a bare dev-server URL in prose, or one wrapped in angle brackets, became a link. `Site::build` now reports these links with their file, line, and column, and asks for inline code instead. Addresses inside code spans and code blocks are never links, so they are unaffected. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #173](https://github.com/ifiokjr/leptoskit/pull/173)
- **Rebuild the served docs when the watcher drops events.** Under load, macOS FSEvents and Linux inotify drop events and report only a rescan that names no file. `Site::serve` ignored those, so an edit could fail to rebuild the site or reload open pages. Rescans and watcher errors now count as changes, and the server also compares the book's file sizes and modification times every second, so an edit no watcher reported still rebuilds. The output directory is created before watching so its writes are never mistaken for edits, which could otherwise rebuild in a loop after a failed first build. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #154](https://github.com/ifiokjr/leptoskit/pull/154)
- **Isolate concurrent function specification exports.** Concurrent CLI exports sharing a Cargo target now use separate temporary output directories. A server that writes no specification reliably reports the missing export, and timeout cleanup joins the server before releasing its output directory. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #169](https://github.com/ifiokjr/leptoskit/pull/169)

#### Re-export JNI bindings from the Android backend

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #78](https://github.com/ifiokjr/leptoskit/pull/78)

`leptoskit_android` (and `leptoskit::android`) now re-exports `jni`, so an Activity's JNI entrypoints can construct `AndroidBackend` with the same `jni` version the backend uses, without declaring the dependency themselves.

The gallery example renders its shared screens with every native backend: AppKit, GTK, Win32, UIKit, and Android Views. CI snapshots every screen on each desktop OS and fails unless the iOS app and the Android instrumentation report every screen as verified.

- **Keep GTK text input placeholders inside their frames.** GTK text inputs no longer clip their text or placeholder vertically. Yoga sizes an input from its text, but the GTK theme gave every `GtkEntry` a 34px minimum height and the frame placed that taller entry from its top edge, so the centered placeholder was pushed down and cut off at the bottom of the frame, as in the gallery's settings search field. Entries now drop the theme's minimum height, and a native control taller than its frame is centered on it, so clipping removes chrome rather than text. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #99](https://github.com/ifiokjr/leptoskit/pull/99)
- **Pin generated CLI output with insta snapshot tests.** The CLI's integration tests now capture everything it generates — scaffolded projects file by file, help text for every subcommand, error wording, and code push release artifacts including packed manifests — in committed insta snapshots, so any change to generated output arrives as a reviewable `.snap` diff instead of passing silently. Test projects moved into a `tests/fixtures` directory shared by the test suites, and the insta dependency gained the `filters` and `redactions` features for the `[ROOT]` path stand-ins and the manifest timestamp redaction. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #24](https://github.com/ifiokjr/leptoskit/pull/24)
- **Render canonical widget names in generated macOS hosts.** The AppKit host accepts the canonical widget names serialized by the Rust renderer and retains legacy aliases, restoring the generated application's controls after the shared component update. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #50](https://github.com/ifiokjr/leptoskit/pull/50)
- **Cache validated Wasm export handles and measure initialized Rust/Wasm/C-ABI plugin round trips, including owned replies and allocation accounting.** The benchmark excludes OS work and UI scheduling and does not establish performance superiority over other frameworks. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #49](https://github.com/ifiokjr/leptoskit/pull/49)
- **Prune native hit testing with cached subtree bounds.** `NativeSurface::hit_test` skips subtrees that cannot contain the point. Each layout commit caches, per node, the union of its frame and its unclipped descendants' frames, so a query reads that dense cache instead of every node's props and layout. Results are unchanged: clipping, rounded clipping, hidden subtrees, stopped activity indicators, and decorative `Pressable` content all hit exactly as before. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #77](https://github.com/ifiokjr/leptoskit/pull/77)
- **Preserve explicit line breaks in native text layout.** Measure every newline-delimited line independently, retaining consecutive and trailing empty lines. Native layouts reserve their combined height while preserving word wrapping, font scaling and the widest line's width. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #172](https://github.com/ifiokjr/leptoskit/pull/172)
- **Exercise registered plugins with a generated Android instrumentation suite, browser lifecycle checks and an interpreted binary-reply fixture.** Document operation-level platform coverage, registration modes, bridge limits and remaining Expo/Tauri/Electron scope. Private validation examples are excluded from release package registration. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #55](https://github.com/ifiokjr/leptoskit/pull/55)
- **Time out native waits whose deadline expires mid-response.** A native UI wait that runs out while reading a driver's response body now fails with the wait's `Error::Timeout`. Previously, reqwest reported that read's timeout as an I/O error, so a slow driver on a loaded host failed the wait with `request or response body error` instead of naming the condition that never appeared. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #123](https://github.com/ifiokjr/leptoskit/pull/123)
- **Preserve generated host compatibility with widget wire names.** Generated UIKit and Android hosts accept the canonical widget names serialized by the Rust renderer, while retaining legacy names. This restores initial interface rendering after the shared component serialization update. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #44](https://github.com/ifiokjr/leptoskit/pull/44)

#### Leave room for native chrome around unstyled text inputs

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #124](https://github.com/ifiokjr/leptoskit/pull/124)

An unstyled `TextInput` or `PasswordInput` with `h-auto` was exactly as tall as its text, so its frame clipped the top and bottom of the native text-field border, such as GTK's entry border and AppKit's rounded bezel. Text inputs that keep the native chrome now measure 12 layout units taller than their text, which makes an unstyled field 28 tall at the default font size, the same height `PasswordInput` already defaults to. Inputs with their own background, border, or corner radius draw no native chrome and still measure from their text and padding, and an explicit height class such as `h-5` still wins.

`NativeProps::is_custom_drawn` reports whether props replace a control's native chrome, the rule the GTK and AppKit backends already applied.

- **Scaffold web projects against a local SDK.** `leptoskit new --sdk-path <repo>` now works for web projects as well as native ones: the client depends on the repository's `crates/leptoskit` by path, so a scaffold can use features that are not released yet. Previously the flag was rejected for web projects, and a fresh web scaffold could only build against the published release. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #81](https://github.com/ifiokjr/leptoskit/pull/81)
- **Remove Android Button's native padding before applying shared layout frames so compact buttons paint their labels.** _Packages:_ 🟢 _leptoskit_bridge_ffi_, 🟢 _leptoskit_cli_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #172](https://github.com/ifiokjr/leptoskit/pull/172)
- **Separate Android service scheduling, native request jobs, widget rendering, and widget properties from application lifecycle control while preserving native callback and shutdown ordering.** _Packages:_ 🟢 _leptoskit_bridge_ffi_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #168](https://github.com/ifiokjr/leptoskit/pull/168)
- **Complete joined guest retirement after cleanup errors, preserve confirmed replacements, and retain recovery diagnostics through fallback.** Guard delayed recovery errors against stale application generations. _Packages:_ 🟢 _leptoskit_bridge_ffi_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #168](https://github.com/ifiokjr/leptoskit/pull/168)
- **Use the existing AppKit font measurer for macOS application layout so native multiline Text receives the space its unchanged native renderer requires.** _Packages:_ 🟢 _leptoskit_bridge_ffi_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #172](https://github.com/ifiokjr/leptoskit/pull/172)
