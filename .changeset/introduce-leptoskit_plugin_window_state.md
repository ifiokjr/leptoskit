---
leptoskit_plugin_window_state: breaking
---

# leptoskit_plugin_window_state: window state

`leptoskit_plugin_window_state` is the native-host plugin for validated window placement and explicit native window restoration.

Native hosts construct it explicitly; it is not a guest plugin and installs nothing by default. The portable state and placement API works everywhere; native capture and restore adapters currently ship for macOS, and the host owns persistence and which window is restored.
