# Plan: leptoskit devtools

Status: **approved** (revision 3). Nothing is implemented yet.

Revision 2 added the implementation detail: the protocol, every instrumentation point with its file, the hub, the transports, the tests, and the pull requests. It also simplified the transport: every app talks to one hub that `leptoskit dev` runs, instead of relaying through the browser extension.

Revision 3 records the answers to the open questions:

- The DevTools panel is published to the stores. It becomes a thin shell around the hub's page, so it always matches the CLI that is running.
- State edited in a native app resets when the guest reloads.
- The hub has no per-run token. It defends against web pages instead, by checking `Host`, `Origin`, and `Sec-Fetch-Site`.
- The interface is downloaded on first use, pinned by hash to the exact CLI version.

[Commonplace](extension-plan.md#the-example-commonplace) is the end-to-end demo.

Decisions:

- **D1:** one debugger for every platform. A web page, an iOS simulator, an Android emulator, and a desktop app all speak one protocol, and one interface shows them.
- **D2:** the debugger inspects what leptoskit owns first: the element tree, layout, styles, routes, queries, forms, named state, plugin calls, and server function calls. Leptos's reactive graph comes last, because it needs hooks upstream.
- **D3:** the debugger interface is itself a leptoskit app. It runs as a page the hub serves, as a browser DevTools panel built on the [extension platform](extension-plan.md), and behind tools on `leptoskit mcp`.
- **D4:** devtools exist only in development. A `devtools` cargo feature compiles them in, `leptoskit dev` is the only command that turns it on, and CI proves release builds contain none of it. A cargo feature is the gate, rather than debug assertions, because native guests are always built with release optimisations (`crates/leptoskit_cli/src/native/guest_profile.rs:11`).
- **D5:** every app, on every platform, sends to one loopback hub that `leptoskit dev` runs. Front ends read from the hub. Nothing relays through a browser extension.
- **D6:** the interface's version is the CLI's version. The hub serves the interface that matches it, and the published DevTools panel frames the hub's page rather than bundling its own copy, so a panel from the store never disagrees with the CLI.
- **D7:** the hub trusts only loopback clients and only browser origins it knows, checking `Host`, `Origin`, and `Sec-Fetch-Site`. It has no per-run token while it listens only on loopback; one becomes mandatory if it ever listens on a network address.

## The problem

React, Vue, Svelte, Angular, and Solid ([solid-devtools](https://github.com/thetarnav/solid-devtools)) each have a DevTools extension. You open a tab beside your app, see the component tree, pick an element on the page to find its component, and edit state to see what happens. Leptos has a community component-tree extension ([`leptos_devtools`](https://crates.io/crates/leptos_devtools)), built for versions before `reactive_graph`, and an open request to expose the reactive graph ([leptos#3466](https://github.com/leptos-rs/leptos/issues/3466)).

leptoskit has a harder version of the problem and a better starting point:

- **Harder:** most of a leptoskit app runs outside a browser. On iOS, Android, and the desktop, the interface is a Wasm guest inside `wasmi` drawing real widgets, so there is no DOM and no browser DevTools to attach to.
- **Better:** leptoskit owns the element tree, the Tailwind resolution, the Yoga layout, the router, the query cache, the forms, the plugin bridge, and the dev process every app already runs under. All of that can be inspected without changing Leptos.

## What it shows

In build order:

1. **Element tree.** The committed `Element` tree: each node's kind, classes, props, accessibility role and label, and the file and line that built it, with "open in editor".
2. **Pick an element.** Hover the running app, in the browser or on a device, to highlight a node and select it in the tree.
3. **Layout and styles.** For the selected node:
   - its classes, and the declarations each class resolved to;
   - its frame, padding, margin, and gap;
   - which responsive and dark-mode variants apply.

   Editing a class re-resolves and re-commits it live.
4. **Route.** The current `Route`, the history stack, and a field that navigates to any typed route.
5. **Queries.** The `leptoskit_query` cache, in the style of TanStack Query's devtools: keys, status, data, staleness, and observers. Actions: refetch, invalidate, and edit data.
6. **Forms.** Each tracked `leptoskit_form`: values, errors, touched state, and submit status.
7. **State.** Signals the app names with `devtools::expose`, readable and editable. On native, a hot reload of the guest resets them like the rest of the guest's state (see "Hot reload").
8. **Calls.** A network-tab-style timeline:
   - plugin bridge calls: plugin, method, status, duration, bytes, cancellation;
   - `leptoskit_rpc` calls: attempts and error class.
9. **Commits.** A profiler of render commits: mutations per commit, time spent in reconcile and layout, and the nodes that changed.
10. **Leptos components and the reactive graph.** Component boundaries, then signals, memos, and effects, once upstream hooks exist (phase 6).

## The cross-platform twist

The hub sees every running platform at once, because every app reports to it. So the debugger can show **one element on every platform at the same time**. Select a node on the web, and the iOS simulator, the Android emulator, and the macOS app highlight the matching node, while the layout panel shows their frames side by side.

Browser layout and Yoga layout differ, and so do font metrics across platforms, and today those differences are found by eye. A side-by-side frame comparison finds them in seconds. Nodes match across platforms by their match key: the source location that built them, plus their index among siblings built at the same location.

## Architecture

```text
 web page (devtools feature)                         leptoskit dev
┌──────────────────────────┐   HTTP + long-poll   ┌──────────────────────────┐   SSE + HTTP   ┌─────────────────────┐
│ agent in the client wasm ├─────────────────────▶│ hub (loopback)           ├───────────────▶│ hub page (UI app)   │
└──────────────────────────┘                      │  sessions, state mirror, │                │ DevTools panel      │
 native app (debug host)                          │  command queues          │◀───────────────┤ leptoskit mcp tools │
┌──────────────────────────┐   HTTP + long-poll   │                          │                └─────────────────────┘
│ host agent: tree, frames,├─────────────────────▶│                          │
│ commits, bridge calls    │                      └──────────────────────────┘
│   ▲ devtools plugin      │
│ guest agent: route,      │
│ queries, forms, state    │
└──────────────────────────┘
```

### Crates

- **`leptoskit_devtools_protocol`:** serde types only, with no platform code. It is shared by agents, the hub, the interface, and MCP.
- **`leptoskit_devtools`:** the agent. Feature `web` is for browser clients; `guest` for native guests; `host` for native hosts and the headless surface used in tests.
- **The hub:** `crates/leptoskit_cli/src/devtools/`, inside the CLI.
- **The interface:** a standalone leptoskit project in `devtools/`, outside the workspace like `examples/file_routes`, built for `web` (CSR) and `extension`.

### The protocol

Every message carries `PROTOCOL_VERSION`. The hub rejects a session on another version with an error that names both versions.

- **Session:** `Hello { app, platform, device, protocol, capabilities }` opens one. Capabilities list which panels this session can feed; a web session has no bridge calls, for example.
- **Events, agent to hub:**
  - `Snapshot`, `TreePatch`, `Commit`, `RouteChanged`;
  - `QueryChanged`, `MutationChanged`, `FormChanged`, `StateChanged`;
  - `Call`, `RpcAttempt`, `Picked`.
- **Commands, hub to agent:**
  - `Select`, `Highlight { key }`, `Pick { on }`, `SetClasses`, `Navigate { href }`;
  - `InvalidateQuery`, `RefetchQuery`, `SetQueryData`, `SetState`;
  - `StartProfiling`, `StopProfiling`, `RequestSnapshot`.
- **Node identity:**
  - A `NodeRef` is a session-local id: the surface `NodeId` on native, or the path on the web.
  - A `MatchKey` is the cross-platform key above.
- **Values:** JSON with their Rust type name. Values over 64 KiB are truncated with a marker. Edits round-trip through the same serde codec the value was sent with, and anything without a codec is read-only.

### The agent's budget

Devtools must not change what they measure:

- **Bounded buffers.** Events go into a bounded queue of 256 events and 4 MiB, the same scale as the plugin bridge's event bounds. On overflow, the oldest patches are dropped and counted, and the next flush sends a fresh snapshot.
- **Off the hot path.** Serialization and HTTP run off the render path: on the host's dev thread on native, and in an idle callback on the web.
- **Profiling costs nothing until asked for.** Commit timing uses `Instant` only while profiling is on.
- **The interaction benchmark must still pass.** The tap-within-a-frame benchmark in `benchmarks/native` must stay within budget with devtools on. CI runs it in both configurations.

## Instrumentation points

Every hook below compiles to nothing without its crate's `devtools` feature.

| What               | Where                                                                                                                                                                                                                                                                                                                                                                                   |
| ------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Source locations   | `crates/leptoskit_native/src/element.rs:45`: add `source: Option<&'static Location<'static>>` to `Element`. `#[track_caller]` on `Element::view/text/button/…` (`element.rs:313-379`) and on the components that wrap them (`components.rs:76,81,108,172` and the rest), so the location is the app's call site. Nothing captures locations today.                                      |
| Commits            | `crates/leptoskit_native/src/surface/reconcile.rs:22` (`commit_source`) and `surface/layout.rs:17` (`commit_layout`): one commit produces two `backend.apply` batches today. The hook records both as one `Commit`, with mutation counts and, while profiling, the time spent in each. It is an `Option<Box<dyn FnMut(&CommitRecord)>>` observer on `NativeSurface` (`surface.rs:185`). |
| Frames and picking | `NativeSurface::frame` (`surface.rs:345`) and `layout` (`:392`) already exist. The private `hit_areas` (`:196`) gains a read accessor for viewport-space highlight rectangles. `hit_test` (`surface/events.rs:97`) answers picking.                                                                                                                                                     |
| Style explanation  | `crates/leptoskit_native/src/tailwind/mod.rs:151,206`: add `Tailwind::explain`, which lists each active class with its declarations (`Rule.declarations` is private today) and the variant that made it active.                                                                                                                                                                         |
| Web nodes          | `crates/leptoskit_native/src/web/mod.rs:133` (`render_node`): add `data-lk-node` with the node's path, and keep the latest tree for each render root. No attribute links DOM nodes back to elements today.                                                                                                                                                                              |
| Guest to host      | `crates/leptoskit_native/src/guest/tree.rs:67` gives each node a wire id. `WireNode` (sent by `node_set_props`) carries `source` when the guest has `devtools`. On the host, `WasmHost::drain_commit` and `resync_map` (`crates/leptoskit_wasm/src/lib.rs:1027,1076`) map wire ids to `NodeId`s, which the host agent reuses.                                                           |
| Plugin calls       | `crates/leptoskit_bridge/src/lib.rs`: `submit` (`:395`), `next_request` (`:423`), `complete` and `complete_reply` (`:437,443`), `take_result` (`:498`), `cancel` (`:521`). Record the time at each step; `Request` (`:146`) has no timestamps today.                                                                                                                                    |
| Server calls       | `crates/leptoskit_rpc/src/lib.rs:272` (`call_with_retry`): one `RpcAttempt` per attempt, with the `FnError` class (`:86`).                                                                                                                                                                                                                                                              |
| Queries            | Already inspectable: `QueryClient::query_infos` (`crates/leptoskit_query/src/client.rs:615`), `subscribe` (`:625`), and `subscribe_mutations` (`mutation_cache.rs:483`). Editing data reuses the dehydration codec the broadcast already uses (`broadcast.rs`). Queries without a codec are read-only.                                                                                  |
| Route              | `Router` (`crates/leptoskit_router/src/router.rs:36`) has no observer API. The app calls `devtools::track_router(&router)` once; the agent watches the `current` signal and calls `navigate` (`:110`) for commands.                                                                                                                                                                     |
| Forms              | `FormState` (`crates/leptoskit_form/src/state.rs:27`) has no serde and no subscription. The app calls `devtools::track_form("signup", &form)`; values are shown through `Debug` and are read-only in phase 4.                                                                                                                                                                           |
| Named state        | `leptoskit_use` hooks return unnamed signals. `devtools::expose("cart.count", signal)` registers a signal whose value is `Serialize + DeserializeOwned`, which makes it editable. Naming is explicit, and unnamed state is not shown.                                                                                                                                                   |

## The hub

`leptoskit dev` starts the hub unless `--no-devtools` is passed. The hub:

- binds `127.0.0.1:7340` by default (the MCP HTTP server uses 7331); `--devtools-port` changes it;
- refuses non-loopback connections;
- writes `.leptoskit/devtools.json` with its URL and process ID, so `leptoskit mcp` can find it.

Agents talk HTTP with a long-poll, the same way native apps already fetch development guests (`crates/leptoskit_cli/src/dev/guests.rs:74`, 25-second waits). That works through `adb reverse` and needs no WebSocket client in the native hosts.

| Route                                         | Purpose                                                         |
| --------------------------------------------- | --------------------------------------------------------------- |
| `POST /agent/sessions`                        | `Hello`; returns the session ID                                 |
| `POST /agent/sessions/{id}/events`            | A batch of events                                               |
| `GET /agent/sessions/{id}/commands?after=<n>` | Long-poll for commands                                          |
| `GET /api/sessions`                           | Sessions, for front ends                                        |
| `GET /api/sessions/{id}`                      | The hub's mirrored state for one session                        |
| `GET /api/stream`                             | Server-sent events of every change                              |
| `POST /api/sessions/{id}/commands`            | Send a command; `Highlight` can target every session of the app |
| `GET /`                                       | The interface                                                   |

The hub mirrors each session's latest state, so a front end that connects late, or MCP, gets everything at once. Sessions end when their long-poll stops for 60 seconds, which happens when the app closes or reloads.

### Who can talk to the hub

A loopback port is not private from the web. Any page open in the developer's browser can send requests to `127.0.0.1`, and a DNS-rebinding page can even read the answers. Without defenses, a malicious page could read the app's state from the hub or send it `SetState` and `Navigate` commands. The hub closes that with checks that need no secret (D7):

- **`Host`** must be `127.0.0.1:<port>`, `[::1]:<port>`, or `localhost:<port>`. A rebinding attack arrives with the attacker's host name, so it is refused with `403`.
- **`Origin`**, when a request has one, must be the hub's own origin, the app's dev server origin, the published panel's extension IDs, or the dev panel's fixed ID. Firefox gives each install a random `moz-extension://` ID, so any `moz-extension:` origin is allowed. Native agents and MCP send no `Origin`, because they are not browsers.
- **`Sec-Fetch-Site: cross-site`** is refused unless the `Origin` is allowed. That catches requests a page makes without CORS, such as an image or a form.
- **Every `POST` requires `Content-Type: application/json`.** A page therefore cannot send a "simple" cross-origin request that skips the CORS preflight, and the hub answers preflights only for allowed origins. It never sends `Access-Control-Allow-Origin: *`.

**What a per-run token would add.** A per-run token is a random secret that `leptoskit dev` creates each time it starts. Every app and front end would have to present it, so only programs started by this run could talk to the hub. The checks above already stop web pages. The token would also stop other programs and other user accounts on the same machine. That is a smaller risk on a development machine, and the token would have to reach the web agent without a rebuild on every run. So the hub has no token while it listens only on loopback. If it ever listens on a network address, for example so a physical phone on Wi-Fi can reach it, the token becomes mandatory and travels the same way as the hub URL.

## Transports

- **Web.**
  - The CLI sets `LEPTOSKIT_DEVTOOLS_HUB` when it builds with `devtools`. `leptoskit_build::platforms()` re-emits it as a rustc environment variable, with `rerun-if-env-changed`, so the default port never invalidates cargo's cache.
  - The agent in the client wasm posts to the hub directly and sets `window.__LEPTOSKIT_DEVTOOLS_SESSION__` to its session ID.
  - Picking draws an overlay element and reads `data-lk-node`.
- **Native.**
  - The CLI passes the hub URL the same way it passes the development guest URL:
    - `LEPTOSKIT_DEVTOOLS` for desktop apps, and through `SIMCTL_CHILD_` for the iOS simulator;
    - `--es leptoskit.devtools` plus `adb reverse` on Android (`crates/leptoskit_cli/src/dev/native.rs:570-625`).
  - The host agent lives in `leptoskit_bridge_ffi` next to `development.rs` (`lk_dev_open`/`lk_dev_next`, `:52,98`) and uses the same HTTP client and background thread.
  - Like development guests, it runs only in debug host builds; `lk_dev_open` already fails in release builds. That is a second gate behind the cargo feature.
  - The host owns the `NativeSurface`, so it reads the tree, frames, commits, and bridge calls directly.
  - The guest reports what only it knows (route, queries, forms, named state) through a `devtools` runtime plugin on the existing bridge. The plugin crate is `leptoskit_plugin_devtools`, shaped like `leptoskit_plugin_logging`, and is selected automatically in devtools builds. The host merges guest and host events into one session.
  - Highlighting needs the Swift and Kotlin renderers to draw an overlay rectangle. That is one new FFI callback, `lk_app_devtools_overlay`, rather than a change to the render transaction format (`crates/leptoskit_bridge_ffi/src/app/render.rs`).
  - While picking is on, the host takes taps for `hit_test` instead of delivering them to the guest.

## Front ends

- **The hub page.** `leptoskit dev` prints its address next to the app's. It needs no installation, which makes it the way to debug a phone.
  - Implemented: the interface in `devtools/` (a leptoskit app built with `devenv shell devtools:build`) shows sessions grouped by app, the element tree with search and picking, node details with "open in editor" (`vscode://file/…`, resolved against the project root from `GET /api/hub`), class editing, the styles, route, queries, mutations, forms, state, calls, and commits panels, guest reloads on the timelines, and the selected node's frame on every other platform with "Highlight everywhere". Panels a session's `Hello.capabilities` don't cover say so.
  - The hub serves it at `/` with `Content-Security-Policy: frame-ancestors 'self' chrome-extension: moz-extension:` (`FRAME_ANCESTORS` in `crates/leptoskit_cli/src/devtools/ui.rs`), from `LEPTOSKIT_DEVTOOLS_UI`, else the downloaded build in `<user cache>/leptoskit/devtools/<version>/`, else the plain `page.html` with a note on how to get the interface.
  - The stream also carries `{kind: "opened", info}` and `{kind: "closed", session}`, so front ends see sessions start and end without polling. `SessionView`, `SessionState`, and the stream's `Change` live in the protocol crate, so the hub and the interface apply events the same way.
- **The DevTools panel.**
  - A small shell built with leptoskit's extension platform, using a `devtools_page` surface that calls `devtools.panels.create("leptoskit", …)`.
  - The panel frames the hub's page, `http://127.0.0.1:<port>/?embed=panel`, instead of bundling the interface (D6). Stores forbid remotely hosted code inside an extension, but framing a page is allowed, and the framed page runs as a web page, not with the extension's privileges.
  - This way, the version a user installed from a store never matters: the interface always comes from the CLI that is running. The shell needs a new store release only when the shell itself changes, which is rare.
  - The shell finds the session by evaluating `window.__LEPTOSKIT_DEVTOOLS_SESSION__` in the inspected tab, and passes it to the framed page with `postMessage`, addressed to the hub's origin.
  - **The message contract** (implemented in `devtools/src/hub.rs`):
    1. On load, the framed page posts `{type: "leptoskit-devtools-ready"}` to `window.parent`. It carries no data, so its target origin is `*`.
    2. The shell answers with `{type: "leptoskit-devtools-session", session: "<id>"}`, and may send it again whenever the inspected tab's session changes.
    3. The page accepts that message only when `event.source === window.parent`, and shows that session, waiting for it if the hub hasn't reported it yet.
    4. If no session is named within 1.5 seconds, the page shows the newest web session, and still switches to the named one when it appears.
  - The hub allows the frame with `Content-Security-Policy: frame-ancestors` listing the panel origins from D7.
  - When no hub is running, the shell shows how to start one, and a port field for a hub on another port.
  - The interface's tab list also shows the native sessions, so a web developer can jump to a simulator from the same panel.
  - The panel is published to the Chrome Web Store, Microsoft Edge Add-ons, and addons.mozilla.org with `leptoskit publish` ([publishing plan](publishing-plan.md)), and asks only for `devtools` and the loopback host permissions.
- **MCP.** `leptoskit mcp` gains these tools in `tools()` and `call_tool` (`crates/leptoskit_cli/src/mcp.rs:202,242`):
  - `devtools_sessions`
  - `inspect_tree` (with depth and a subtree root)
  - `inspect_node`
  - `explain_styles`
  - `query_cache`
  - `navigate`
  - `highlight`
  - `set_state`

  They read `.leptoskit/devtools.json` and call the hub API. Today the only tree an MCP tool can read is the Appium page source (`capture_source`, `crates/leptoskit_testing/src/control/mod.rs:902`). These tools give agents leptoskit's own tree and layout on any platform.

## Hot reload

On native, `leptoskit dev` replaces the guest when its code changes. The new guest starts from scratch, so its state resets: queries, forms, and every value edited in the debugger. Edited values are not carried across, because a value edited for the old code may not fit the new code.

The debugger keeps its own place across the reload:

- The host agent keeps the session open and sends a fresh `Snapshot` once the new guest commits.
- The interface marks the reload on the calls and commits timelines.
- The interface keeps the selected node selected when its match key still exists in the new tree.

On the web, a reload is a new page and a new session, and the interface switches to it when the app and the platform match.

## Feature wiring

- `crates/leptoskit/Cargo.toml`: `devtools = ["dep:leptoskit_devtools", "leptoskit_native/devtools", "leptoskit_query?/devtools", "leptoskit_router?/devtools", "leptoskit_form?/devtools", "leptoskit_rpc?/devtools", "leptoskit_bridge/devtools"]`.
- Scaffold app manifest (`crates/leptoskit_cli/src/new_project.rs:581`): `devtools = ["leptoskit/devtools"]`. Existing apps get one line, which `leptoskit doctor` suggests when it is missing.
- `leptoskit dev` adds `devtools` to `cargo_features` (collected in `plugin_context`, `crates/leptoskit_cli/src/build.rs:545`) for web clients and native guests. `leptoskit build` never adds it, and errors if `build.features` contains it together with `--release`.
- Switching between `leptoskit dev` and `leptoskit build` changes the feature set and makes cargo rebuild. The web build already separates targets per platform (`.leptoskit/web/target`). Phase 1 measures whether devtools builds need their own target directory too.

## Distributing the interface

The CLI cannot build the interface from source on every machine, because that needs the whole wasm toolchain. So it downloads it, and the version is always the CLI's own (D6):

1. **Built once per release.** `.github/workflows/publish.yml` gains a job that runs before `upload-assets`. It builds `devtools/` once, uploads `leptoskit-devtools-<version>.tar.zst` to the draft release for tag `v<version>`, and outputs the archive's SHA-256.
2. **The hash is compiled into the CLI.** The binary builds in `upload-assets` receive the hash as `LEPTOSKIT_DEVTOOLS_UI_SHA256`, and the CLI's build script embeds it, with `rerun-if-env-changed`. The crate published to crates.io carries the same hash in a `devtools-ui.sha256` file, written before packaging, so `cargo install` gets it too.
3. **Downloaded on first use.** `leptoskit dev` downloads the archive for exactly its own version from that release into `<user cache>/leptoskit/devtools/<version>/`, and checks the hash. It never falls back to another version: the protocol and the interface change together. A hash mismatch is an error and is never retried silently. `leptoskit devtools fetch` downloads it ahead of time, for CI and for working offline.
4. **Offline.** The hub still runs and MCP still works. The page explains how to fetch the interface later.
5. **Built from source.** A CLI built from a git checkout has no hash. Inside this repository, `LEPTOSKIT_DEVTOOLS_UI=<path>` serves a local build of `devtools/`. Anywhere else, the hub serves an explanation instead of the interface.

Trust follows the CLI binary itself: the hash comes from the same release the binary was installed from, so the download cannot be swapped for another one.

Implemented:

- `crates/leptoskit_cli/build.rs` embeds `LEPTOSKIT_DEVTOOLS_UI_SHA256`, or the first word of `crates/leptoskit_cli/devtools-ui.sha256`, as `LEPTOSKIT_DEVTOOLS_UI_HASH`, and refuses anything that isn't a hex SHA-256.
- `crates/leptoskit_cli/src/devtools/fetch.rs` downloads `leptoskit-devtools-<version>.tar.gz` (gzip: zstd isn't a dependency) from `v<version>`, checks the hash, unpacks only plain files and directories that stay inside the archive, into a temporary directory beside `<user cache>/leptoskit/devtools/<version>/`, and renames it into place. `LEPTOSKIT_DEVTOOLS_RELEASES` replaces the GitHub base for a mirror; the hash still applies.
- `leptoskit devtools fetch [--force]`; `leptoskit dev` downloads on a background thread when a hash is compiled in and the cache is empty, and the hub picks the build up on the next page load.
- Tests against a local fake release server: success and replacement, a wrong hash, a missing release, another version's archive, an archive that isn't the interface.
- `devenv shell devtools:build [out]` builds `devtools/` into `<out>/ui` and writes the archive and its `.sha256`. The publish workflow job that runs it is pending as a patch (the token can't push workflow changes).

The DevTools panel extension is published to the stores (D6, and phase 2 below).

## Tests

- **Protocol.** Serde round trips, version mismatch errors, truncation markers.
- **Source locations.** `View()` and every component record the caller's file and line. A compile test checks that `Element` is unchanged in size without the feature.
- **Commits.** A `HeadlessBackend` surface with the observer records one `Commit` per `set_root`, `set_viewport`, and press, and the records are snapshot with insta.
- **Styles.** `Tailwind::explain` snapshots for responsive, dark, and state variants.
- **Web.** `wasm-bindgen-test` checks that `data-lk-node` appears only with the feature, and that picking maps a DOM node back to the right element.
- **Bridge.** Timeline events for submit, run, complete, cancel, and late completion, with the same fixtures the bridge suites use.
- **Hub.** Axum tests for sessions, long-poll wake-ups and timeouts, mirroring, version rejection, session expiry, loopback refusal, and SSE.
- **End to end, headless.** An in-process `WasmHost` running the `hooks_guest` example with `devtools`, reporting to a hub on an ephemeral port. Assert the tree, a commit, a guest query event, and a `Navigate` command round trip.
- **End to end, platforms.** The macOS app in CI under `leptoskit dev`, checked through `leptoskit mcp`'s `inspect_tree`. The iOS simulator and Android emulator jobs gain the same check in phase 3.
- **End to end, Commonplace.** Commonplace's website, iOS app, and extension under one `leptoskit dev`, reporting three sessions to one hub. Selecting a clip card on the website highlights it in the simulator, once phase 3 lands.
- **Hub security.** Requests with a foreign `Host`, a foreign `Origin`, `Sec-Fetch-Site: cross-site`, or a `POST` without `Content-Type: application/json` are refused. A real browser page on another origin cannot read `/api/sessions` or send a command. That is checked with `wasm-bindgen-test` in Chrome for Testing against a hub on an ephemeral port.
- **Hot reload.** A headless guest is replaced mid-session: the session survives, a new `Snapshot` arrives, and a value set with `SetState` before the reload is back to its default after it.
- **Distribution.** The download rejects a wrong hash, a missing release, and another version's archive, against a local fake release server.
- **MCP.** Dispatch tests for every new tool against a fake hub.
- **Release gate.** CI builds a scaffolded app with `leptoskit build --release` for web and for a native guest, and fails if the wasm contains a `leptoskit_devtools` symbol or the hub URL string.
- **Performance.** The `benchmarks/native` interaction benchmark runs with and without devtools, and both must meet the budget.

## Phases and pull requests

Each pull request carries a changeset. Dependent layers go in a `gh stack`.

**Phase 0: protocol, hub, and the tree on macOS and headless.** Does not need the extension platform.

1. `feat(devtools)`: `leptoskit_devtools_protocol`.
2. `feat(native)`: source locations, the commit observer, frame access, and `Tailwind::explain`, behind `devtools`.
3. `feat(devtools)`: the host agent in `leptoskit_devtools` for `NativeSurface`, and the in-process headless test.
4. `feat(dev)`: the hub in `leptoskit dev`, with its `Host`, `Origin`, and `Sec-Fetch-Site` checks, `.leptoskit/devtools.json`, `--no-devtools`, and the release gate in CI.
5. `feat(devtools)`: the `devtools/` interface with the tree, node details, and styles panels, served by the hub, plus the macOS host agent in `leptoskit_bridge_ffi`. **Interface and serving done**; native snapshots carry classes and their declarations (`Node.styles`).
6. `ci`: the interface archive in the publish workflow, the hash compiled into the CLI, and `leptoskit devtools fetch`. **The hash and `leptoskit devtools fetch` are done; the publish workflow job is written and waiting for a token with the `workflow` scope.**

**Phase 1: the web.**

1. `feat(native)`: `data-lk-node` and the web agent.
2. `feat(devtools)`: picking and highlighting on the web, and the route panel.

**Phase 2: the DevTools panel.** Depends on phases 1 and 2 of the [extension plan](extension-plan.md).

1. `feat(extension)`: the `devtools_page` surface and `leptoskit_extension::devtools`.
2. `feat(devtools)`: the panel shell, framing the hub page, and inspected-tab session matching.
3. `ci`: publish the panel with `leptoskit publish` from the release workflow when the shell changes. This depends on phase 1 of the [publishing plan](publishing-plan.md).

**Phase 3: iOS and Android, and the twist.**

1. `feat(plugins)`: `leptoskit_plugin_devtools` and the guest agent.
2. `feat(native)`: hub URLs passed to the simulator and emulator, and the iOS and Android host agents.
3. `feat(native)`: `lk_app_devtools_overlay` in the Swift and Kotlin renderers, and picking on native.
4. `feat(devtools)`: highlighting across platforms by match key, and side-by-side frames.

**Phase 4: data panels.**

1. `feat(query)`: the queries panel with refetch, invalidate, and edit.
2. `feat(devtools)`: forms, named state, and editing state.
3. `feat(bridge)`: the calls timeline, plugin and rpc. **Done:** `leptoskit_bridge::devtools` and `leptoskit_rpc::devtools` (`drain`, `dropped`, `now_ms`) behind each crate's `devtools` feature, recording into a shared `leptoskit_devtools_protocol::sink::Sink` of 256 records. `CallRecord` gained optional `running_ms`, `taken_ms`, `completed_late_ms`, `request_bytes`, and `response_bytes`. Still to do: the agents drain these sinks into `Event::Call` and `Event::RpcAttempt`, the facade's `devtools` feature turns both on, and the interface draws the timeline.
4. `feat(devtools)`: the commits profiler.

**Phase 5: MCP.**

1. `feat(mcp)`: the devtools tools. **Done:** all eight in `crates/leptoskit_cli/src/mcp/devtools.rs`, with dispatch tests against a fake hub. `explain_styles` shows each node's active classes and the declarations the session resolved them to (the protocol's `Node::styles`); a session that sends only classes gets a note saying so. `highlight` with `all_sessions` fans out from the MCP side, since the hub's command route targets one session.

**Phase 6: Leptos internals.**

1. `feat(devtools)`: Leptos component boundaries from its `tracing` feature, which nothing in the workspace enables today. Measure the cost first.
2. Upstream: a proposal to `reactive_graph` for observer hooks behind a feature (leptos#3466). leptoskit does not fork `reactive_graph`; the signal graph view waits for upstream.

## Decided

- **Q1, publishing the panel:** yes. It is published to the Chrome Web Store, Edge Add-ons, and addons.mozilla.org, as a shell that frames the hub's page (D6).
- **Q2, hot reload on native:** edited state resets with the rest of the guest's state ("Hot reload").
- **Q3, a per-run token:** none while the hub listens only on loopback. `Host`, `Origin`, and `Sec-Fetch-Site` checks stop web pages instead, and a token becomes mandatory if the hub ever listens on a network address (D7).
- **Q4, shipping the interface:** downloaded on first use from the GitHub release for the exact CLI version, and checked against a hash compiled into the CLI ("Distributing the interface").
- **Q5, physical phones:** USB and simulators first: `adb reverse` on Android, and the simulator on iOS. A network listener, with the mandatory token, is later work.
