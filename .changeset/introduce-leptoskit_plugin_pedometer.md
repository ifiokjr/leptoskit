---
leptoskit_plugin_pedometer: major
---

# leptoskit_plugin_pedometer: step counts

`leptoskit_plugin_pedometer` is the leptoskit plugin for bounded, demand-driven step-counter readings with explicit platform count domains.

Add `leptoskit_plugin_pedometer` as a direct dependency and select the `pedometer` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::Pedometer` rather than resolving the plugin on every call. Only iOS (CoreMotion) and Android (its step counter) count steps; the app supplies the motion usage description or activity-recognition permission. iOS reports steps for the requested interval and can read history from the past seven days, while Android reports its cumulative count since boot, so neither is a daily total. macOS, Windows, Linux, and the browser report no sensor and never invent counts.
