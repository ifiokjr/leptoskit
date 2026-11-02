---
leptoskit_plugin_posture: breaking
---

# leptoskit_plugin_posture: foldable posture

`leptoskit_plugin_posture` is the leptoskit plugin for fold state, hinge geometry, and hinge angles.

Add `leptoskit_plugin_posture` as a direct dependency and select the `posture` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::DevicePosture` rather than resolving the plugin on every call. A posture describes the device the app is running on and every hinge that crosses its window; desktop and the browser report a flat, non-foldable device.
