---
leptoskit_plugin_pedometer: breaking
---

# leptoskit_plugin_pedometer: step counts

`leptoskit_plugin_pedometer` is the leptoskit plugin for bounded, demand-driven step-counter readings with explicit platform count domains.

Add `leptoskit_plugin_pedometer` as a direct dependency and select the `pedometer` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::Pedometer` rather than resolving the plugin on every call. iOS uses CoreMotion and Android uses its step counter; the app supplies the motion usage description or activity-recognition permission.
