# Plan: browser extensions as a platform

Status: **approved** (revision 3). Nothing is implemented yet. The work starts with Commonplace, a research app on web, native, and server, and a spike built on it that settles the risky assumptions before the platform work starts.

Revision 2 added the implementation detail: every file each phase touches, the generated output, the tests, and the pull requests. Revision 3 records the answers to the open questions:

- Commonplace is the driving example, and every phase adds its surfaces to it.
- Safari is in scope on macOS and on iOS.
- Publishing to the extension stores is part of the platform, through the [publishing plan](publishing-plan.md).

It also corrects the Safari packaging: the macOS app is assembled from SwiftPM, not an Xcode project.

Decisions:

- **E1:** an extension is a platform, like `ios` or `macos`. It has an `extension` application feature, `.extension.rs` overrides through `platform_module!`, and its own output under `.leptoskit/extension/`. It is not native: it never goes through the native build.
- **E2:** every extension surface opens at a route in the app's existing `Route` tree. The background worker is the only surface without a screen.
- **E3:** one source tree, one wasm build, and one package per browser family: Chromium, Firefox, and Safari. No universal zip.
- **E4:** leptoskit owns its extension API bindings in a new `leptoskit_extension` crate. It does not depend on `web-extensions-rs`.
- **E5:** extensions are opt-in. `leptoskit new --extension` or an `[extension]` table in `leptoskit.toml` turns them on; the default scaffold does not change.
- **E6:** Commonplace (`examples/commonplace`) drives the work. It is one product with a server, a website, native apps, and an extension. Each phase lands with the Commonplace surface that uses it, and Commonplace's end-to-end tests are the platform's end-to-end tests.
- **E7:** Safari ships on macOS and iOS, as an app extension inside the app leptoskit already builds for each.
- **E8:** getting an extension into a store is part of the platform. `leptoskit publish` uploads to the Chrome Web Store, Microsoft Edge Add-ons, and addons.mozilla.org, and sends Safari through App Store Connect with the app. The [publishing plan](publishing-plan.md) covers it, together with the app stores.

The leptoskit debugger, whose DevTools panel is another user of this platform, is planned in [devtools-plan.md](devtools-plan.md).

## The problem

Many apps want a browser extension next to the website and the mobile app: a popup that shows the same account, a side panel that works beside whatever page is open, or an overlay that adds the product to someone else's site. Today that means a second codebase, usually in TypeScript, that duplicates the app's state, its API client, and its components.

leptoskit already writes one interface for the browser and five native platforms. An extension is the same interface in a few new places, so it should come from the same crates.

## The example: Commonplace

Commonplace is a research notebook, named after the commonplace books people kept of passages worth remembering. You keep research projects, such as "Urban heat islands". You clip quotes and pages into a project from wherever you read them, annotate the clips, and export a bibliography. It is the kind of product that needs every surface at once, which is why it drives this plan (E6).

It is a standalone project in `examples/commonplace`, built by the CLI and excluded from the workspace like `examples/opencode/tui`:

```text
examples/commonplace/
├── leptoskit.toml          [db], [auth], [extension]
├── commonplace_db/         tables: projects, sources, clips, notes, tags
├── src/
│   ├── api.rs              #[api] server functions, shared by every surface
│   ├── routes/             file routes: the library, a project, a clip, export, sign-in,
│   │                       plus the extension's /clip, /panel, /reading, /settings, /overlay
│   ├── extension.rs        #[extension::main]
│   └── background.rs       #[extension::background]
├── ios/  macos/  android/  the native hosts; the Apple ones carry the Safari extension
└── tests/                  the end-to-end journeys below
```

The data model:

- **Projects** hold sources, clips, and notes.
- **A source** is a page you read. It records the URL, title, author, site, and the dates it was published and accessed, which is everything a citation needs.
- **A clip** is a quote from a source, with your note and tags. It also stores a W3C Web Annotation text-quote selector (the exact text plus a little context on each side), so the clip can be found and highlighted on the page again.
- **Notes** are markdown and link to clips.

What each surface does:

| Surface                                        | What it does in Commonplace                                                                                                                                                                          | What it exercises                                                     |
| ---------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------- |
| Server                                         | Server functions and the database, and sign-in with email codes from the auth pack                                                                                                                   | `leptoskit_db`, `leptoskit_auth`, `#[api]`                            |
| Website                                        | The full library: projects, search, reading a clip in context, writing notes, exporting APA, MLA, or BibTeX                                                                                          | SSR, file routes, queries, forms                                      |
| Native apps (iOS, Android, macOS)              | The library away from the browser: reading, tagging, and writing notes. The iOS and macOS apps also contain the Safari extension.                                                                    | The shared routes as native widgets; the host projects                |
| Popup (`/clip`)                                | Clip the current page: the title, URL, and selected text are filled in, and you pick a project, add tags, and save                                                                                   | `tabs`, `scripting`, typed messages, server functions from a page     |
| Side panel (`/panel`)                          | The active project beside whatever you are reading. Pressing a clip opens its source and scrolls to the passage.                                                                                     | `side_panel`, query sync between surfaces                             |
| Content script `clipper` (injected on request) | A "Clip" bubble over the selection, injected only when you press the toolbar button or use the shortcut, so it needs no host permission                                                              | `activeTab`, `scripting.executeScript`, the shadow-root mount         |
| Content script `highlights` (registered)       | Highlights your existing clips on pages you have clipped from. It is off until you turn it on for a site, which asks for that site's permission and registers the script at run time.                | Optional host permissions, runtime registration, the port transport   |
| Background worker                              | A "Clip selection" context menu item and a keyboard shortcut. It keeps a queue of clips saved while offline and retries them on an alarm. The badge shows the number of clips from the current page. | Cold starts, `context_menus`, `commands`, `alarms`, `action`, storage |
| Options (`/settings`)                          | Sign in, choose the default project, and pick the sites that show highlights                                                                                                                         | Sign-in from an extension, `permissions`                              |
| New tab (`/reading`)                           | Your reading queue: sources saved for later. Chromium and Firefox only, unless the spike shows Safari supports `chrome_url_overrides`.                                                               | `chrome_url_overrides`, a surface that is not on every browser        |

The extension asks for no host permissions at install: `activeTab`, `scripting`, `storage`, `contextMenus`, `alarms`, the side panel, and the server's origin. A narrow request like that passes store review fastest, and it is the shape most real extensions should copy. That is why the clipper is injected on request rather than declared on `<all_urls>`.

Sign-in from the extension uses a bearer token, which the auth plan already prefers over the cookie (`Authorization: Bearer`, [auth-plan.md](auth-plan.md)). The options page runs the email-code flow through the same server functions as the website, and keeps the session token in `storage.local`, which the background worker can read. Spike question 6 decides whether sharing the website's cookie is simpler on some browsers.

Commonplace also shows off the [debugger](devtools-plan.md): with `leptoskit dev` running the website, the iOS simulator, and the extension, all three report to one hub.

What it is not: a published product. Commonplace is not listed in any store, because a listing needs a hosted server and someone to answer reviews. The release workflow runs `leptoskit publish --dry-run` on it, so every store's checks run on a real extension (see [publishing-plan.md](publishing-plan.md)).

## What already fits

- The web build produces ES-module wasm-bindgen output (`wasm::bindgen`, `crates/leptoskit_cli/src/wasm.rs:502`, with `.web(true)`), and `app.render = "csr"` renders entirely in the browser.
- Plugins choose a browser implementation with `cfg(all(target_arch = "wasm32", target_os = "unknown", not(feature = "guest")))`, for example `crates/leptoskit_plugin_store/src/lib.rs:340`.
- `leptoskit_router` has a memory history, `Router::new` (`crates/leptoskit_router/src/router.rs:79`). A popup has no address bar, so it routes in memory.
- `leptoskit_query` syncs caches over `BroadcastChannel` (`crates/leptoskit_query/src/broadcast.rs:97`). Every extension page shares one origin, so the popup, side panel, and options page share query state.
- `platform_module!` accepts any lowercase platform name (`crates/leptoskit_macros/src/platform_module.rs:113`), so `home.extension.rs` works as soon as the build sets `LEPTOSKIT_PLATFORM=extension`.
- `leptoskit::web::render` emits each tree's stylesheet next to its nodes (`crates/leptoskit_native/src/web/mod.rs:67`), so a tree mounted in a shadow root keeps its styles inside it.
- Plugins already declare what native hosts need in `[package.metadata.leptoskit.runtime]` (`android-permissions`, `apple-usage-descriptions`). Extension permissions follow the same pattern.
- `native/harmonyos.rs:26` and `native/rust_desktop.rs:34` already refuse code push for platforms that cannot use it.

## What the app author writes

### Configuration

This is Commonplace's configuration:

```toml
[extension]
name = "Commonplace" # defaults to app.name
browsers = ["chromium", "firefox", "safari"]
server = "https://commonplace.example.com" # server functions and host permission
permissions = [
	"activeTab",
	"scripting",
	"contextMenus",
	"alarms",
] # added to what plugins derive
host_permissions = [] # added to `server` and to `manifest` content scripts' matches
optional_host_permissions = [
	"https://*/*",
] # asked for at run time, one site at a time

[extension.popup]
route = "/clip"

[extension.side_panel]
route = "/panel" # left out of the Safari package, which has no side panel

[extension.options]
route = "/settings"

[extension.new_tab]
route = "/reading"
browsers = ["chromium", "firefox"] # a surface can narrow the browser list

[[extension.content_scripts]]
name = "clipper"
route = "/overlay/clipper"
inject = "on_request" # packaged, but only injected by `scripting`

[[extension.content_scripts]]
name = "highlights"
route = "/overlay/highlights"
inject = "registered" # registered at run time for the sites the user allows

[extension.background]
events = [
	"runtime.message",
	"runtime.installed",
	"alarms",
	"context_menus",
	"commands",
]

[extension.commands.clip]
suggested_key = "Alt+Shift+C"
description = "Clip the selection"

[extension.firefox]
id = "commonplace@leptoskit.dev"

[extension.safari]
bundle_id_suffix = "SafariExtension" # appended to native.package_id
```

A content script's `inject` is one of:

- `manifest` (the default): declared in the manifest with `matches` and `run_at`, so it needs host permissions for every match.
- `on_request`: packaged and listed in `web_accessible_resources`, but injected only when the app calls `extension::scripting::inject(tab, "clipper")`. With `activeTab`, it needs no host permission.
- `registered`: packaged and registered at run time with `extension::scripting::register("highlights", matches)`, after the user grants those matches through `extension::permissions::request`.

Every surface except `background` is optional, and at least one must be present. A surface that a browser cannot show is left out of that browser's package when the surface narrows its own `browsers`. Validation (`LeptoskitConfig::validate`, `crates/leptoskit_config/src/lib.rs:1226`) rejects:

- a surface that a listed browser cannot show, unless the surface narrows its `browsers`. `side_panel` is the exception: the Safari package leaves it out automatically, and `extension::side_panel::open` returns `Unavailable` there. Commonplace then opens `/panel` in a tab instead;
- `firefox` without `[extension.firefox].id`;
- `safari` without `native.package_id`, because the Safari extension's bundle identifier is derived from the app's;
- an empty configuration;
- `codepush.enabled = true` together with `extension` in `app.platforms`.

### The entry point

The app crate adds one function under its `extension` feature:

```rust,ignore
#[leptoskit::extension::main]
fn extension(surface: Surface) -> impl IntoView {
	match surface {
		Surface::Page { route } => app_at(route.parse::<Route>().expect("configured route")),
		Surface::Content { name, route } => overlay(name, route),
	}
}

#[leptoskit::extension::background]
async fn background(events: Events) {
	while let Some(event) = events.next().await { … }
}
```

`#[extension::main]` generates `#[wasm_bindgen] pub fn leptoskit_extension_start(surface: &str, route: &str)`. That function mounts the view with a memory-history router and sets the server function URL from `extension.server`. `#[extension::background]` generates `leptoskit_extension_background()`, which receives the events the JS shim queued.

The build checks every configured `route` against the routes `leptoskit routes` generated, so a typo fails the build rather than the popup.

### Messages

Surfaces and the background worker share serde types from the app's shared crate:

```rust,ignore
// commonplace's shared module
#[derive(Serialize, Deserialize)]
pub enum Message { Clip { url: String, title: String, quote: TextQuote, project: ProjectId } }

// background: saves through the server function, or queues while offline
extension::runtime::on_message(|message: Message, sender| async move { … });

// the clipper content script, or the popup
extension::runtime::send::<Message, Reply>(Message::Clip { url, title, quote, project }).await?;
```

## What the build produces

`leptoskit build --platform extension` runs the existing client pipeline (`build::run`, `crates/leptoskit_cli/src/build.rs:105`) with the `extension` feature and `csr`, then packages once per browser family:

```text
.leptoskit/extension/
├── chromium/
│   ├── manifest.json
│   ├── popup.html  popup.js  side_panel.html  side_panel.js  options.html  options.js
│   ├── background.js
│   ├── reading.html  reading.js
│   ├── content-clipper.js  content-highlights.js
│   ├── pkg/commonplace.js  pkg/commonplace_bg.wasm
│   └── assets/
├── firefox/  (same layout, Firefox manifest)
├── safari/   (no side panel or new tab; copied into the Apple app extensions)
├── chromium.zip
├── firefox.zip
└── firefox-source.zip  (the source archive addons.mozilla.org asks for; see below)
```

Rules for the generated files:

- **No inline scripts.** Extension pages forbid them, so each page is a static HTML file that loads its own `<surface>.js` module. The app's `web/index.html` is not used.
- **Pages.** `popup.js` imports the bound JS, awaits `init()`, and calls `leptoskit_extension_start("popup", "/clip")`.
- **Background worker.**
  - Module service workers cannot use top-level `await`, so `background.js` starts `init()` without awaiting it.
  - It registers every listener from `extension.background.events` synchronously on the first turn, queues events until the wasm is ready, and then calls `leptoskit_extension_background()`.
  - `runtime.onMessage` replies through a promise in Firefox and Safari, and through `sendResponse` plus `return true` in Chromium; the shim hides that.
- **Content scripts.**
  - Content scripts cannot be ES modules, so `content-clipper.js` is a classic script. It dynamically imports `runtime.getURL("pkg/…js")` and passes the wasm URL to `init()`.
  - It mounts into a closed shadow root on a host element, with `app.css` adopted into that root.
  - The `pkg/` files become `web_accessible_resources` for the content script's `matches` only, or for `<all_urls>` for `on_request` and `registered` scripts, whose pages are not known at build time. Chromium also gets `use_dynamic_url: true`, so pages cannot fingerprint the extension by a fixed URL.
  - Only `manifest` scripts appear under `content_scripts`. The other two are files that `scripting` injects or registers by name, and the generated `content_scripts.json` maps each name to its file.
- **Manifests.**
  - Chromium gets `background.service_worker` with `"type": "module"`.
  - Firefox gets `background.scripts` with `"type": "module"`, plus `browser_specific_settings.gecko` with `id` and `strict_min_version` 121.
  - `side_panel` maps to `side_panel` in Chromium and `sidebar_action` in Firefox.
- **Permissions** are the union of:
  - `extension.permissions`;
  - the `extension-permissions` metadata of every plugin crate in the app's dependency graph, read from `cargo metadata`, which the build already loads;
  - `storage` when `store` is present;
  - `side_panel` when configured.

  Host permissions are the union of `extension.host_permissions`, the origin of `extension.server`, and the `matches` of every `manifest` content script. `optional_host_permissions` is copied as it is; Firefox treats every host permission as optional in MV3 anyway.
- **Content security policy.** The manifest declares `"script-src 'self' 'wasm-unsafe-eval'; object-src 'self'"` for extension pages.
- **Release builds** run `wasm-opt` as the web build does, and skip precompression, because stores compress the zip themselves.
- **The zips are deterministic:** sorted entries and fixed timestamps, so the same source produces byte-identical packages.
- **A source archive for Firefox.** addons.mozilla.org asks for the source of any machine-generated code, and wasm compiled from Rust counts. The build writes `firefox-source.zip`, containing the app's crates, `Cargo.lock`, `rust-toolchain.toml`, `leptoskit.toml`, and a generated `BUILD.md` with the exact commands and tool versions that reproduce `firefox.zip`. It leaves out `target/`, `.leptoskit/`, `.env*`, and everything `.gitignore` excludes. `leptoskit publish` uploads it with the add-on.
- **Versions.** `manifest.json`'s `version` comes from `app.version` with the prerelease and build parts dropped, because stores accept only one to four dot-separated integers. `version_name` keeps the full string on Chromium.

## How different the browsers are

They share the [WebExtensions API](https://developer.mozilla.org/en-US/docs/Mozilla/Add-ons/WebExtensions) and Manifest V3. The differences are in packaging and in a small set of APIs:

| Area                 | Chromium (Chrome, Edge, Brave, Opera, Arc, Vivaldi) | Firefox                                                   | Safari                                  |
| -------------------- | --------------------------------------------------- | --------------------------------------------------------- | --------------------------------------- |
| Namespace            | `chrome.*`, promises in MV3                         | `browser.*`, promises; `chrome.*` alias                   | `browser.*` and `chrome.*`              |
| Background           | `background.service_worker`                         | `background.scripts` (event page); no service workers     | Service worker or non-persistent page   |
| Side surface         | `side_panel` and `chrome.sidePanel`                 | `sidebar_action`                                          | None                                    |
| Extension ID         | Assigned by the store                               | `browser_specific_settings.gecko.id` required             | Bundle identifier of the containing app |
| Host permissions     | Granted at install                                  | Optional by default in MV3; the user grants them later    | The user grants them per site           |
| Packaging and review | One zip for every Chromium store                    | Signed by addons.mozilla.org, including self-distribution | Ships inside a signed macOS or iOS app  |
| Remotely hosted code | Banned                                              | Banned                                                    | Banned                                  |

Firefox still does not support `background.service_worker` ([bug 1573659](https://bugzilla.mozilla.org/show_bug.cgi?id=1573659)). Since Chrome 121 and Firefox 121, a manifest may list both keys, and each browser ignores the one it does not use ([MDN](https://developer.mozilla.org/en-US/docs/Mozilla/Add-ons/WebExtensions/manifest.json/background)). That makes one Chromium-and-Firefox zip possible, but it is not worth it:

- Firefox needs a gecko ID that the Chrome Web Store warns about.
- Side panels use different keys.
- Safari cannot share the zip at all.

So each family gets its own generated manifest (E3). Background code must run both as a worker and as an event page, so it never calls `importScripts` and never assumes a DOM.

## Safari on macOS and iOS

Safari runs a web extension only from inside an app, as an app extension with the `com.apple.Safari.web-extension` extension point. leptoskit already builds an app for both Apple platforms, so the Safari extension rides inside it (E7). On iOS it is the same app that users already install from the App Store.

The two hosts are built differently today, so they get the extension differently:

- **iOS.** The CLI writes `.leptoskit/ios/Host.xcodeproj` by hand (`crates/leptoskit_cli/src/native/project/ios.rs:50`). It gains a second target, `SafariExtension.appex`, embedded in `App.app` through an "Embed Foundation Extensions" build phase:
  - the extension's resources are `.leptoskit/extension/safari/`;
  - a generated `Info.plist` declares `NSExtension` with the extension point and `SafariWebExtensionHandler` as the principal class;
  - a generated `SafariWebExtensionHandler.swift` answers `browser.runtime.sendNativeMessage`. It is empty at first, and a later revision can route it to the app's plugins;
  - the bundle identifier is `native.package_id` plus `.` plus `extension.safari.bundle_id_suffix`. Apple requires the extension's identifier to start with the app's.
- **macOS.** The macOS app is not an Xcode project. `native/desktop.rs` builds the Swift package and assembles the bundle by hand (`build`, `:87`; `assemble`, `:130`), then signs it ad hoc (`:117`). The extension follows the same pattern:
  - `swift build` builds a second product, the extension's executable;
  - `assemble` writes `Contents/PlugIns/SafariExtension.appex` with its `Info.plist` and resources;
  - it signs the extension with the App Sandbox entitlement, which Safari requires of app extensions, before signing the app. The app itself stays unsandboxed.

  The spike checks that Safari loads an extension from a bundle assembled this way. If it does not, the fallback is to generate a macOS Xcode project the way iOS has one, which is a larger change.
- **Turning it on.** Users enable the extension in Safari's settings, so the app has to tell them how. On macOS, the generated app gains an "Open Safari Extension Settings" menu item that calls `SFSafariApplication.showPreferencesForExtension`. iOS has no such call, so Commonplace's iOS app shows the steps on its welcome screen.
- **What Safari lacks.** No side panel, no `chrome_url_overrides` until the spike shows otherwise, and host permissions granted per site by the user. `Unavailable` covers the APIs; the `browsers` key on each surface covers the packaging.
- **iOS limits.** Safari on iOS keeps a tight memory limit on extension background pages. The spike measures Commonplace's background worker on an iPhone, and decides between one wasm for every surface and a smaller background binary (question 5).
- **Development.** `leptoskit dev --platform ios` and `--platform macos` copy the dev Safari build into the extension before the app builds, so the simulator's Safari runs the current code. Safari has no command to reload an extension, so a rebuild reinstalls the app.
- **Without a native app.** App Store Connect now has a Safari Web Extension Packager that takes the extension's files and builds the app around it ([Apple](https://developer.apple.com/documentation/safariservices/packaging-and-distributing-safari-web-extensions-with-app-store-connect)). Developers report that the upload section is missing for some accounts, so leptoskit does not depend on it. For an app with `extension` as its only platform, `leptoskit publish safari` can use it later, as described in the [publishing plan](publishing-plan.md).

## `leptoskit_extension`

[`web-extensions-rs`](https://github.com/web-extensions-rs/web-extensions) supports only Chromium, with Firefox pending, and is a small project. leptoskit needs bindings that:

- behave the same on all three families, binding `browser` when it exists and falling back to `chrome`;
- expose `async fn` over the promise-based MV3 functions, and stop cleanly when dropped;
- return an `Unavailable` error for an API the browser lacks, rather than a JavaScript exception;
- expose events as `Stream`s that remove their listener when dropped.

The crate is plain `wasm-bindgen` and `js-sys`, built only for `wasm32-unknown-unknown`. Where web-extensions-rs or its `-sys` crate already has correct bindings, borrow them with credit, or contribute fixes upstream. Firefox publishes JSON schemas for the whole API (`toolkit/components/extensions/schemas`), so a later revision can generate bindings from them; the first set is written by hand.

| Module                                                                | Contents                                                                                    | Phase |
| --------------------------------------------------------------------- | ------------------------------------------------------------------------------------------- | ----- |
| `surface`                                                             | `Surface`, the `main` and `background` macros' runtime, `Events`, and the shadow-root mount | 1     |
| `runtime`                                                             | `send`, `on_message`, `connect` ports, `on_installed`, `get_url`, `id`, `reload`            | 2     |
| `storage`                                                             | `local`, `sync`, `session`: typed `get`/`set`/`remove` and change streams                   | 2     |
| `tabs`, `windows`                                                     | `query`, `active`, `create`, `update`, `send_message`                                       | 2     |
| `action`                                                              | badge text and color, title, `open_popup`                                                   | 2     |
| `scripting`                                                           | `execute_script`, and registering content scripts at runtime                                | 3     |
| `side_panel`                                                          | Chromium `sidePanel` and Firefox `sidebarAction` behind one API, `Unavailable` on Safari    | 3     |
| `alarms`, `context_menus`, `commands`, `notifications`, `permissions` | Thin typed wrappers                                                                         | 3     |
| `devtools`                                                            | `inspected_window`, `panels`, `network`, for the debugger                                   | 5     |
| `safari`                                                              | `send_native_message` to the containing app                                                 | 4     |

## Code that assumes a page

Some browser code calls `web_sys::window()`, which is `None` in a background worker. All of it already fails safe (unavailable, or disconnected), but some of it should work in a worker:

| Code                                                          | In the background worker                                                                      |
| ------------------------------------------------------------- | --------------------------------------------------------------------------------------------- |
| `leptoskit_query/src/broadcast.rs:291`, `managers.rs:139,161` | Use `js_sys::global()`: `BroadcastChannel` exists in workers, so the worker joins query sync. |
| `leptoskit_plugin_http/src/web.rs:33`                         | Use the global `fetch`, which workers have.                                                   |
| `leptoskit_plugin_store/src/web.rs:34`                        | Under `extension`, use `storage.local`, shared by pages and the worker.                       |
| `leptoskit_use/src/persistent/web.rs:42`                      | Pages keep `localStorage`; the worker reports unavailable. Document it.                       |
| `leptoskit_plugin_notifications/src/web.rs`                   | Under `extension`, use the `notifications` API.                                               |
| `leptoskit_plugin_linking/src/web.rs`                         | Under `extension`, use `tabs.create`.                                                         |
| Router, l10n, sensors, dialogs, and other page-only plugins   | Unchanged: they report unavailable in the worker, which has no screen.                        |

A small shared helper resolves the global scope as a window or a worker, so each crate doesn't repeat the check. Content scripts run on the page's origin, so `BroadcastChannel` doesn't reach them. They get query sync through a `BroadcastTransport` over `runtime` ports (`broadcast_with`, `broadcast.rs:106`).

## Server functions and sign-in

There is no `set_server_url` call anywhere in the repository today; native apps reach the server through the HTTP plugin. Extensions are CSR builds that include the server function client, so `leptoskit_extension_start` calls `server_fn::client::set_server_url(extension.server)` before mounting. The server's origin is added to `host_permissions` automatically. Generated RPC clients already take `with_server_url` (`crates/leptoskit_rpc_codegen/src/rust.rs:651`), so they get the same value.

Sign-in uses a bearer token by default, like the native apps. The auth layer already resolves `Authorization: Bearer` before the cookie ([auth-plan.md](auth-plan.md)):

- The options page runs the sign-in flow through the app's own server functions.
- `leptoskit_extension` keeps the session token in `storage.local`, where every surface and the background worker can read it.
- The server function client sends the token on every call.

Two alternatives stay available:

- **Share the website's session cookie.** This works when the browser sends first-party cookies on requests from an extension that has host permissions for the site. The spike checks this per browser, and Commonplace uses it where it works, so someone already signed in on the website is signed in to the extension.
- **`identity.launchWebAuthFlow`**, for third-party login providers. The auth plan owns that decision; this plan only provides the API.

## Developing

`leptoskit dev` gains `extension` in its platform selection (`Selection`, `crates/leptoskit_cli/src/dev/native.rs:44`) as a third kind next to `web` and `native`:

1. **Build.** It builds `.leptoskit/extension/dev/chromium/` without `wasm-opt`. The dev manifest adds:
   - a fixed `key`, so the extension ID is stable across runs;
   - a host permission for `http://127.0.0.1/*`.
2. **Launch.** It starts Chrome for Testing with a dedicated profile in `.leptoskit/extension/profile/` and `--load-extension`. Branded Chrome 137 and later ignore that flag, which is why it uses Chrome for Testing; `leptoskit doctor` checks it is installed.
3. **Reload.** The dev background shim long-polls the dev server for the build hash. It uses the same `?after=<hash>` long-poll the native guest route uses (`crates/leptoskit_cli/src/dev/guests.rs:76`) and calls `runtime.reload()` when the hash changes. Open extension pages close and reopen on the next click; that is how browsers reload extensions.
4. **Firefox.** In phase 5, `leptoskit dev` installs the add-on temporarily through WebDriver BiDi `webExtension.install`. Until then it prints the `about:debugging` steps.

## Code push

The extension platform refuses code push in three places:

- `LeptoskitConfig::validate`;
- the extension build, with the same `ensure!` the HarmonyOS and desktop builds use;
- `validate_release_platform` (`crates/leptoskit_cli/src/codepush.rs:398`), so `leptoskit codepush release --platform extension` fails with a message that names the store policy.

## Every file this touches

| Area      | Files                                                                                                                                                                                                                                                                                                                                                                                                                  |
| --------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Config    | `crates/leptoskit_config/src/lib.rs`: add `Platform::Extension` and its arms in `feature`, `is_native` (false), `Display`, `FromStr`, and the `UnknownPlatform` text (`:186-290`, `:1447`); add `ExtensionConfig` as `Option` like `db`; validation in `:1226`. `schema.rs` patches. `docs/src/schema*.json` through `schema:update`. No migration step: the table is additive.                                        |
| Build     | `crates/leptoskit_build/src/lib.rs:10`: add `"extension"` to `PLATFORMS`.                                                                                                                                                                                                                                                                                                                                              |
| CLI build | `crates/leptoskit_cli/src/build.rs`: the `is_native` hand-off stays false for extension (`:77`); `builds_client` and the render-mode gates force `csr` (`:171-279`); plugin generate and serve take the platform instead of a hard-coded `Platform::Web` (`:462,529`). New `crates/leptoskit_cli/src/extension/{mod,config,manifest,pages,package,safari}.rs`.                                                         |
| Wasm      | `crates/leptoskit_cli/src/wasm.rs:114`: `with_platform` adds `csr` alongside `extension`.                                                                                                                                                                                                                                                                                                                              |
| Dev       | `crates/leptoskit_cli/src/dev.rs` and `dev/native.rs:44-130,322,353`: the `Selection` kind, `support`, the Chrome for Testing launch, and the reload route.                                                                                                                                                                                                                                                            |
| Codepush  | `crates/leptoskit_cli/src/codepush.rs:378,398`.                                                                                                                                                                                                                                                                                                                                                                        |
| Scaffold  | `crates/leptoskit_cli/src/new_project.rs`: a `--extension` flag in `cli.rs:166` following `--db` (`:112,355,525`); the app's `extension` feature in `app_manifest` (`:581`); `src/extension.rs` and `src/background.rs` templates; readme and verify lists (`:830-885`).                                                                                                                                               |
| Plugins   | `crates/leptoskit_cli/src/native/registry.rs:64`: accept `extension-permissions` in plugin metadata. The `web.rs` changes in the table above, each behind its crate's `extension` feature.                                                                                                                                                                                                                             |
| SDK       | `crates/leptoskit/Cargo.toml`: `extension = ["web", "csr", "dep:leptoskit_extension"]` and the `leptoskit::extension` re-export. New `crates/leptoskit_extension` and `crates/leptoskit_extension_macros`. Both are added to `monochange.toml`.                                                                                                                                                                        |
| Apple     | `crates/leptoskit_cli/src/native/project/ios.rs`: the `SafariExtension` target, its build phases, and the embed phase in the generated `project.pbxproj` and scheme. `native/desktop.rs:17,87,130`: the extension product in the generated `Package.swift`, the `.appex` in `assemble`, and its sandboxed signature. New templates `native/templates/SafariExtensionInfo.plist` and `SafariWebExtensionHandler.swift`. |
| Doctor    | `crates/leptoskit_cli/src/doctor.rs`: Chrome for Testing, and Xcode for Safari.                                                                                                                                                                                                                                                                                                                                        |
| Docs      | New `docs/src/platforms/extension.md` and a guide chapter; the platform lists in `readme.md`, `docs/src/platforms/readme.md`, and `docs/src/readme.md` "Where things stand"; `cli.rs` help text ("all six").                                                                                                                                                                                                           |

## Tests

- **Config.** Parsing, defaults, and every validation error. Update the tests that assert `is_native() == (p != Web)` (`crates/leptoskit_config/tests/config.rs:136`) and the `APPLICATION` comparisons (`crates/leptoskit_cli/tests/cli.rs:236,423`).
- **Manifests.** Insta snapshots of the generated `manifest.json` for each browser family across a matrix: popup only, every surface, each `inject` mode, plugins that add permissions, a surface narrowed to some browsers, and a dev build. Commonplace's three manifests are snapshot too, because they are the ones a store reviewer reads.
- **Pages and shims.** Snapshots of every generated HTML and JS file, plus a test that no HTML file contains an inline script.
- **Packages.** Building twice gives byte-identical zips, and every manifest validates against the browsers' published manifest schemas. A test unpacks `firefox-source.zip`, follows its `BUILD.md`, and checks that the result matches `firefox.zip`; it runs in the nightly job because it is a full build.
- **Scaffold.** New `new_extension__*` snapshots, added to the pinned list in `crates/leptoskit_cli/tests/snapshots.rs:176`; the existing `new_web__*` snapshots must not change (E5).
- **Bindings.** `wasm-bindgen-test` in a real browser for the namespace fallback, `Unavailable` results, and listener removal on drop, with a fake `browser` object for paths the test browser lacks.
- **Commonplace without the extension.** Server function tests against a test database, and journeys on the website and the headless native backend: sign in, create a project, clip a source by hand, write a note, and export a bibliography.
- **End to end.** There is no harness that loads an unpacked extension today (`crates/leptoskit_cli/src/testing.rs` runs `wasm-bindgen-test` through WebDriver). Add `leptoskit test extension`:
  - It drives Chrome for Testing through chromedriver with `--load-extension`, and Firefox through geckodriver with BiDi `webExtension.install`.
  - It runs Commonplace's journeys against a local Commonplace server and a fixture site served with a strict content security policy:
    - sign in from the options page, and the popup shows the account;
    - select text on the fixture page, clip it through the toolbar button, and find the clip on the website;
    - clip through the context menu after the background worker has been stopped, which checks a cold start;
    - stop the server, clip, start it again, and watch the offline queue drain on the next alarm;
    - allow highlights for the fixture site, reload it, and see the clip highlighted;
    - edit a clip in the side panel, and see the popup update without a refetch, which checks query sync.
- **Safari.** macOS CI builds Commonplace for macOS and for the iOS simulator with the Safari extension embedded. It checks that each `.appex` is in place, has the right bundle identifier, and passes `codesign --verify`. Running in Safari is checked by hand on a Mac and an iPhone for every phase 4 pull request, and recorded in [platform-validation.md](platform-validation.md).
- **Publishing.** `leptoskit publish --dry-run` runs on Commonplace in CI for every store. It covers the [publishing plan](publishing-plan.md)'s checks without uploading anything.

## Phases and pull requests

Each pull request carries a changeset. Dependent layers go in a `gh stack`. Each phase also adds its surfaces to Commonplace, in the same pull request as the feature they use, so the example never shows something the platform cannot do yet.

**Phase 0a: Commonplace without the extension.** Built only from what leptoskit ships today: `leptoskit_db`, the auth pack, `#[api]`, file routes, queries, and forms.

1. `feat(examples)`: the Commonplace server, database, and sign-in, with server function tests.
2. `feat(examples)`: the website: library, projects, clips, notes, and bibliography export.
3. `feat(examples)`: the native apps on iOS, Android, and macOS, from the same routes.

Gaps in the existing features that Commonplace finds become issues, and fixes land before phase 1 when they block it.

**Phase 0b: spike.** Hand-written manifests and pages around Commonplace's CSR build, loaded unpacked in Chrome, Firefox, Safari on macOS, and Safari on an iPhone. Record the answers here:

1. The bound wasm loads in a popup on all three families and on iOS.
2. A content script injected with `activeTab` can dynamically import its module and compile wasm on github.com, which has a strict content security policy.
3. Background workers start cold correctly: listeners registered synchronously, events queued until wasm is ready. Measure the cold-start time.
4. Firefox loads `background.scripts` with `"type": "module"`.
5. Bundle size and memory: one wasm for every surface, or a separate background binary. Safari on iOS decides this, because of its memory limit.
6. Whether the site's session cookie is sent from an extension page with host permissions, per browser.
7. Safari on macOS loads an extension from an app bundle assembled without Xcode, as `native/desktop.rs` builds it.
8. Which of `chrome_url_overrides`, `commands`, and `contextMenus` Safari supports.

The spike's code is thrown away. Its output is the answers, recorded here, and decisions on 5 and 7.

**Phase 1: the platform and its pages.** One stack:

1. `feat(config)`: `Platform::Extension`, `ExtensionConfig`, validation, schema.
2. `feat(extension)`: `leptoskit_extension` with `surface`, plus `leptoskit_extension_macros` with `main`.
3. `feat(cli)`: manifests, pages, packages, the Firefox source archive, and `leptoskit build --platform extension` for Chromium and Firefox; popup, options, and new tab. Commonplace gains `/clip` (pasting a URL by hand for now), `/settings` with bearer sign-in, and `/reading`.
4. `feat(cli)`: `leptoskit new --extension`, the minimal starter beside Commonplace.

**Phase 2: APIs and the background worker.**

1. `feat(extension)`: `runtime`, `storage`, `tabs`, `windows`, `action`, and typed messages. The popup fills in the current tab's title and URL, and the badge counts clips.
2. `feat(extension)`: the `background` macro and the worker shim, and Commonplace's offline queue.
3. `feat(plugins)`: `extension-permissions` metadata, and the `store`, `http`, `notifications`, and `linking` backends.
4. `feat(query)`: `BroadcastChannel` in workers.

**Phase 3: side panels and content scripts.**

1. `feat(extension)`: `side_panel`, `scripting`, `alarms`, `context_menus`, `commands`, `notifications`, `permissions`. Commonplace gains `/panel`, the context menu, and the shortcut.
2. `feat(cli)`: content scripts in all three `inject` modes, shadow-root mounting, and `web_accessible_resources`. Commonplace gains the clipper and highlights.
3. `feat(query)`: the runtime-port broadcast transport for content scripts.

**Phase 4: Safari.**

1. `feat(cli)`: the Safari package, and the Safari extension target in the generated iOS Xcode project.
2. `feat(cli)`: the Safari extension in the assembled macOS app, or a generated macOS Xcode project if spike question 7 says so.
3. `feat(examples)`: Commonplace's Safari extension on both, and the welcome screen that explains how to turn it on.

**Phase 5: developing and testing.**

1. `feat(dev)`: `leptoskit dev` builds, launches, and reloads the extension; Chrome for Testing in `leptoskit doctor`.
2. `test(cli)`: `leptoskit test extension`, Commonplace's journeys, and their CI job.
3. `feat(dev)`: Firefox through BiDi `webExtension.install`, and the Safari build in `leptoskit dev --platform ios` and `macos`.
4. `docs`: the public platform page, a guide chapter that walks through Commonplace, and the "Where things stand" entries.

Phase 5's dev loop is useful from phase 1 on, so its first pull request can move earlier once phase 1 lands.

**Phase 6: publishing.** The extension half of the [publishing plan](publishing-plan.md), whose phases 1 and 2 cover `leptoskit publish` for the Chrome Web Store, Edge Add-ons, and addons.mozilla.org, and Safari through App Store Connect.

## Decided

- **Q1, the first example:** Commonplace, a full product with a server, a website, native apps, and an extension, rather than a small page highlighter (E6).
- **Q2, Safari on iOS:** in scope, together with macOS (E7).
- **Q3, store publishing:** leptoskit publishes, and makes it as easy as it can. It covers the extension stores first and the app stores after them, in its own plan (E8).

## Open questions

- **Q4:** Should Commonplace become a live, hosted demo with store listings, as a showcase? The proposal is no for now: the dry runs in CI prove the publishing path without a server to run or reviews to answer.
