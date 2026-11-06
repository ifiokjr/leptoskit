---
leptoskit_plugin_screen_orientation: major
---

# leptoskit_plugin_screen_orientation: orientation requests

`leptoskit_plugin_screen_orientation` is the leptoskit plugin for scoped activity, scene, and browser-document orientation requests with explicit restoration.

Add `leptoskit_plugin_screen_orientation` as a direct dependency and select the `screen-orientation` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::ScreenOrientation` rather than resolving the plugin on every call. iOS and Android hold the policy for the app's own scene or activity and restore the previous one when the hold is dropped, browser apps use `web::hold`, and macOS, Windows, and Linux have no native orientation lock. A pending hold is not an acknowledgement that the OS rotated; the snapshot reports the presented interface, not accelerometer orientation.
