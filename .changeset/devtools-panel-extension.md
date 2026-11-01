---
"release": minor
---

# A DevTools panel in the browser's developer tools

leptoskit DevTools gains a **leptoskit** panel in the developer tools of Chrome, Edge, and Firefox, built on the extension platform, and every extension can now add a panel of its own.

- **`[extension.devtools_page]`.** A new extension surface with two routes: `route`, which the browser's developer tools open out of sight for each tab, and `panel`, the panel it adds. The build declares `devtools_page` for Chromium and Firefox and writes `devtools.html` and `devtools_panel.html`; Safari has no developer tools API, so its package leaves them out. `PageName` gains `Devtools` and `DevtoolsPanel`, and `leptoskit test extension` checks both pages.
- **`leptoskit::extension::devtools`.** `create_panel`, `inspected_tab`, `eval`, and `on_navigated` for the inspected page, and, for leptoskit's own hub, `watch_session`, which follows `window.__LEPTOSKIT_DEVTOOLS_SESSION__` across reloads, and `HubFrame`, which frames the hub's interface and passes it the inspected tab's session.
- **The panel.** `devtools/` built as an extension is a small shell: it finds the hub on `127.0.0.1:7340`, or a port you enter and it remembers, frames the hub's interface instead of bundling it, so it always matches the running `leptoskit dev`, and says how to start a hub when none is running.
- **The hub.** Chromium can now show it in the panel: `frame-ancestors` also allows `devtools:`, the browser's own DevTools page that contains the panel, and a cross-site navigation to the interface's pages is no longer refused. Requests from other sites to the API still are.
