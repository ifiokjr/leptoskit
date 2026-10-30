---
"release": minor
---

# Browser extensions

An app can now be a browser extension for Chromium browsers, Firefox, and Safari, built from the same routes as its website and native apps. Add `extension` to `app.platforms`, describe the surfaces in `[extension]`, and run `leptoskit build --platform extension`.

- **Surfaces.** The popup, the side panel (Firefox's sidebar), the options page, the new tab page, and content scripts each open one of the app's routes. The build fails when a route doesn't exist. The background worker registers the browser's listeners on its first turn, as cold starts require, and queues events until the app's wasm is ready.
- **Content scripts** run the app in a frame of the extension's own page, inside a closed shadow root, because most large sites forbid compiling WebAssembly in their pages. `leptoskit::extension::page` reads the page's details and selection, places the frame, and highlights quotes. A content script can be declared in the manifest, injected on request with `activeTab`, or registered at run time for the sites the user allows.
- **`leptoskit::extension`** (the `extension` feature) has `entry!`, typed messages between surfaces and the worker, and `storage`, `tabs`, `action`, `scripting`, `permissions`, `alarms`, `context_menus`, `commands`, `notifications`, and `side_panel`. Each works the same in every browser, returns `Error::Unavailable` where a browser lacks it, and stops listening when dropped.
- **Packages.** One folder and one deterministic zip per browser family, each with its own `manifest.json`, plus the source archive addons.mozilla.org asks for. Firefox's data collection declaration comes from `[extension.firefox].data_collection`.
- **Sign-in from another origin.** A request with `Authorization: Bearer` and no session cookie passes the same-site check from any origin, and another origin may ask for a code and sign in with `"transport": "bearer"`. Signing in for a cookie still has to come from the app's own site.
