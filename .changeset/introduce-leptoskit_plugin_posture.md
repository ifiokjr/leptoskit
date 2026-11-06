---
leptoskit_plugin_posture: major
---

# leptoskit_plugin_posture: foldable posture

`leptoskit_plugin_posture` is the leptoskit plugin for fold state, hinge geometry, and hinge angles, so a layout can put content on either side of a fold.

Add `leptoskit_plugin_posture` as a direct dependency and select the `posture` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::DevicePosture` rather than resolving the plugin on every call, and browser apps use the `web` module. iOS 27.1 uses the hinge interaction API, Android uses Jetpack WindowManager and the hinge angle sensor, HarmonyOS (experimental) uses its display fold status, and Chromium browsers report the fold without angles. macOS, Windows, and Linux report the plugin unavailable.
