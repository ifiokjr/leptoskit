---
"release": minor
---

# Persist shared application state across native and web

`leptoskit_use` now supplies platform-neutral async storage hooks and optional persistent JSON storage: native application files and browser local storage implement the same contract. Async saves and reloads serialize per handle, preserve the last successful value on error, and report owner disposal instead of updating dead signals. Native async file operations run off the UI thread without requiring a specific runtime.

The `leptoskit` `storage` feature enables these adapters. Viewport and breakpoint hooks also accept fractional logical pixels, matching the native Tailwind renderer. Shared-screen tests save and reopen preferences through both native and browser controls.
