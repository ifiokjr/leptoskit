# leptoskit_plugin_global_shortcut

Scoped global shortcuts using the maintained [Tauri global-hotkey backend](https://github.com/tauri-apps/global-hotkey), following [Tauri's global-shortcut API](https://v2.tauri.app/plugin/global-shortcut/) and [Electron's explicit registration conflicts](https://www.electronjs.org/docs/latest/api/global-shortcut).

Create `GlobalShortcuts` after native application startup on the event-loop thread. macOS requires the main thread; Windows requires that thread's Win32 message loop. Linux supports X11 only. Native Wayland sessions, browsers and mobile explicitly return `Unavailable`; there is no misleading XWayland fallback or browser keyboard listener masquerading as a global shortcut.

`register("Control+Shift+KeyK")` returns a `Registration` guard. Dropping it unregisters; explicit `unregister` is idempotent and returns native failures. Conflicts never replace an existing shortcut. Up to 64 registrations and 128 queued events are supported. `drain` reports overflow with a loss counter, so callers can reset held-key assumptions. RAII cleanup failures are available through `take_cleanup_error`.

The native manager and guards are not Send or Sync. A guard keeps the manager alive until unregister. This crate exclusively owns the process-wide `global-hotkey` event callback; do not use that dependency separately elsewhere in the application.

Use `with_waker` to post event-loop wake messages from native callbacks. Wakeups coalesce until the next drain. The callback may run on another thread and must be nonblocking. A callback panic is caught before crossing native frames and reported by `take_cleanup_error` (or the next guest event poll). The callback must not directly access UI or enter Wasm. The host decides when to drain/present events.

`ShortcutPlugin` exposes registration, unregister, and bounded event polling over the binary bridge; enable `guest` for the typed Wasm client. Hosts install and grant it explicitly after constructing a native manager. Automatic generated registration is intentionally absent because construction is fallible and requires an initialized event loop.

Tests use an injected backend to verify guards, conflicts and overflow without stealing a real keyboard shortcut or requiring OS permissions. Native dependency compilation does not establish that a particular user-selected accelerator is available.
