---
leptoskit_plugin_global_shortcut: breaking
---

# leptoskit_plugin_global_shortcut: global shortcuts

`leptoskit_plugin_global_shortcut` is the native-host plugin for scoped desktop global shortcuts with explicit registration conflicts and bounded events.

Native hosts construct it explicitly; it is not a guest plugin and installs nothing by default. macOS, Windows, and X11 are supported; native Wayland, browsers, and mobile return `Unavailable`, with no XWayland fallback or fake browser shortcut.
