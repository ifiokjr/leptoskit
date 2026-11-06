---
leptoskit_plugin_global_shortcut: major
---

# leptoskit_plugin_global_shortcut: global shortcuts

`leptoskit_plugin_global_shortcut` is the leptoskit plugin for scoped desktop global shortcuts with explicit registration conflicts and bounded events.

The native host creates `GlobalShortcuts` on its event-loop thread after startup; it has no `native.plugins` entry and installs nothing by default. `register` returns a guard that unregisters when dropped, and a host can wrap the manager in `ShortcutPlugin` so a guest with the `guest` feature registers shortcuts and polls events through `guest::GlobalShortcut`. macOS, Windows, and X11 are supported; native Wayland, browsers, and mobile return `Unavailable`, with no XWayland fallback or fake browser shortcut.
