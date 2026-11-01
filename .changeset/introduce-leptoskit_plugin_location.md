---
leptoskit_plugin_location: breaking
---

# leptoskit_plugin_location: foreground location

`leptoskit_plugin_location` is the leptoskit plugin for foreground location permission and cancellable bounded fixes through CoreLocation, Android LocationManager, or browser Geolocation.

Add `leptoskit_plugin_location` as a direct dependency and select the `location` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::Location` rather than resolving the plugin on every call. Native reads never prompt; request permission as a separate, intentional action, and the app supplies its usage description and manifest permissions.
