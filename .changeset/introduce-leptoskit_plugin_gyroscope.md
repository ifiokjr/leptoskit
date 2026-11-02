---
leptoskit_plugin_gyroscope: breaking
---

# leptoskit_plugin_gyroscope: gyroscope readings

`leptoskit_plugin_gyroscope` is the leptoskit plugin for cancellable angular velocity readings in radians per second around native device axes.

Add `leptoskit_plugin_gyroscope` as a direct dependency and select the `gyroscope` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::Gyroscope` rather than resolving the plugin on every call. iOS and Android use real system sensors, desktop has no sensor, and the browser converts DeviceMotion degrees to radians.
