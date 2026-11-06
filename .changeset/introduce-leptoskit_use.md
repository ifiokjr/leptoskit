---
leptoskit_use: major
---

# leptoskit_use: cross-platform hooks

`leptoskit_use` provides cross-platform reactive primitives inspired by leptos-use: state, derived values, watchers, timers, storage, and viewport breakpoints without a DOM or executor.

The hooks read leptos signals and accept host services explicitly, so the same code runs in a browser and in a native guest. The `local-storage` feature adds persistent JSON storage in native app directories and the browser's local storage, and the `web` feature adds browser-only hooks such as media queries and event listeners. It is the utility layer beneath app screens, not a UI framework.
