---
leptoskit_plugin_screen_orientation: breaking
---

# leptoskit_plugin_screen_orientation: orientation requests

`leptoskit_plugin_screen_orientation` is the leptoskit plugin for scoped activity, scene, and browser-document orientation requests with explicit restoration.

Add `leptoskit_plugin_screen_orientation` as a direct dependency and select the `screen-orientation` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::ScreenOrientation` rather than resolving the plugin on every call. A pending hold is not an acknowledgement that the OS rotated; the snapshot reports the presented interface, not accelerometer orientation.
