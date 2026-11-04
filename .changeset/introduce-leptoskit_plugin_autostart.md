---
leptoskit_plugin_autostart: major
---

# leptoskit_plugin_autostart: login startup

`leptoskit_plugin_autostart` is the leptoskit plugin for explicit per-user desktop login registration with a validated identity and command: a LaunchAgent on macOS, an XDG autostart entry on Linux, and the current user's Run key on Windows.

The native host constructs `Autostart` itself with the app's identity, executable, and arguments; it has no `native.plugins` entry and installs nothing by default. The host calls `enable`, `disable`, or `is_registered` directly, or registers it on the bridge so a guest with the `guest` feature can use `guest::AutostartClient` without choosing the command. Constructing it never enables login startup, an existing entry it doesn't own is never overwritten, and iOS, Android, and the browser return `Unavailable`.
