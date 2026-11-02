---
leptoskit_plugin_tray: breaking
---

# leptoskit_plugin_tray: tray icons

`leptoskit_plugin_tray` is the native-host plugin for explicitly owned native desktop tray icons with bounded events.

Native hosts construct it explicitly; it is not a guest plugin and installs nothing by default. The host calls `TrayHost::install` after its event loop starts and owns removal; Linux uses the GTK-free KSNI backend, and mobile and the browser have no tray.
