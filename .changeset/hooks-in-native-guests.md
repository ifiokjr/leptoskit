---
"release": patch
---

# Hooks run in native apps

`leptoskit::hooks` and Leptos signals work inside a native app's Wasm runtime on iOS, Android, macOS, Windows, and Linux, not only in the browser. The runtime calls a mounted view again after every event, so screens read their signals' current values; call `leptoskit::guest::render()` after a change that happens outside an event, such as a plugin reply. A compiled guest that uses hooks now runs through the interpreter in CI, so a dependency update that would stop such a guest from loading fails there first.
