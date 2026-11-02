---
leptoskit_plugin_brightness: breaking
---

# leptoskit_plugin_brightness: screen brightness

`leptoskit_plugin_brightness` is the leptoskit plugin for scoped mobile screen brightness overrides with explicit restoration.

Add `leptoskit_plugin_brightness` as a direct dependency and select the `brightness` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::Brightness` rather than resolving the plugin on every call. Android changes only the activity window, iOS changes the screen and restores the previous value, and desktop and the browser return `Unavailable`.
