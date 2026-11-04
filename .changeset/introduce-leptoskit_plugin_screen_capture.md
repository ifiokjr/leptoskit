---
leptoskit_plugin_screen_capture: major
---

# leptoskit_plugin_screen_capture: capture protection

`leptoskit_plugin_screen_capture` is the leptoskit plugin for window-scoped capture prevention and capture observation as separate capabilities.

Add `leptoskit_plugin_screen_capture` as a direct dependency and select the `screen-capture` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::ScreenCapture` rather than resolving the plugin on every call. The plugin never captures pixels or reads screenshots; Android can protect its owned window and, on Android 14 and later, report screenshots; iOS can observe scene capture and app screenshots without prevention; macOS, Windows, Linux, and the browser report both unavailable.
