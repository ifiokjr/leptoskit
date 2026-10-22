# leptoskit_plugin_window_state

Window placement and restoration inspired by [Tauri window-state](https://v2.tauri.app/plugin/window-state/) and [Electron window bounds](https://www.electronjs.org/docs/latest/api/browser-window).

Persist `WindowState` through your application's storage, then call `placement` with current `Display` work areas. Deserialization validates geometry. Disconnected monitors, oversized windows, negative monitor coordinates, and changed work areas are handled without restoring an inaccessible window. Coordinates are logical units; the host supplies all rectangles in one consistent coordinate system.

On macOS, `macos::capture` and `macos::restore` operate on the exact `NSWindow` supplied by its owner. Track normal bounds before maximizing; fullscreen needs the host's asynchronous window lifecycle and returns `Unavailable`. Restoration does not activate the application.

The portable state/placement API works on every target. Native capture/restore adapters for Windows, Linux, browsers, and mobile are not implemented. There is no automatic global-window lookup, guest capability, or implicit persistence. A host deliberately controls which window is restored and when its state is saved.
