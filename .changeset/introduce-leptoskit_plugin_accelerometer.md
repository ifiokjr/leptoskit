---
leptoskit_plugin_accelerometer: breaking
---

# leptoskit_plugin_accelerometer: accelerometer readings

`leptoskit_plugin_accelerometer` is the leptoskit plugin for cancellable acceleration readings including gravity, in metres per second squared, along native device axes.

Add `leptoskit_plugin_accelerometer` as a direct dependency and select the `accelerometer` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::Accelerometer` rather than resolving the plugin on every call. iOS and Android use real system sensors, desktop has no sensor, and the browser uses Generic Sensor or DeviceMotion.
