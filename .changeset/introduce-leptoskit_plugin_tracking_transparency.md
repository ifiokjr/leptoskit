---
leptoskit_plugin_tracking_transparency: breaking
---

# leptoskit_plugin_tracking_transparency: tracking consent

`leptoskit_plugin_tracking_transparency` is the leptoskit plugin for explicit iOS App Tracking Transparency status and authorization requests.

Add `leptoskit_plugin_tracking_transparency` as a direct dependency and select the `tracking-transparency` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::TrackingTransparency` rather than resolving the plugin on every call. It never reads advertising identifiers or starts tracking; iOS returns the system status, and other platforms return `Unavailable`.
