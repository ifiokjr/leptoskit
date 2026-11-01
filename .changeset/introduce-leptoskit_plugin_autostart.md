---
leptoskit_plugin_autostart: breaking
---

# leptoskit_plugin_autostart: login startup

`leptoskit_plugin_autostart` is the native-host plugin for explicit per-user desktop login registration with a validated identity and command.

Native hosts construct it explicitly; it is not a guest plugin and installs nothing by default. Constructing the manager never enables login startup; registration is an explicit call, and mobile, browser, and unsupported desktops return `Unavailable`.
