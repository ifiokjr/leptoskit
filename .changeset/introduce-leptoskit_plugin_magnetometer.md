---
leptoskit_plugin_magnetometer: breaking
---

# leptoskit_plugin_magnetometer: magnetometer readings

`leptoskit_plugin_magnetometer` is the leptoskit plugin for cancellable magnetic-field readings in microteslas along native device axes.

Add `leptoskit_plugin_magnetometer` as a direct dependency and select the `magnetometer` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::Magnetometer` rather than resolving the plugin on every call. This is a magnetic sensor measurement, not a compass heading; platform calibration differs, and desktop has no sensor.
