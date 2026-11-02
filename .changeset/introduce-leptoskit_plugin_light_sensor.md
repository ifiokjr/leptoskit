---
leptoskit_plugin_light_sensor: breaking
---

# leptoskit_plugin_light_sensor: ambient light

`leptoskit_plugin_light_sensor` is the leptoskit plugin for ambient illuminance in lux from Android's light sensor or the browser's AmbientLightSensor.

Add `leptoskit_plugin_light_sensor` as a direct dependency and select the `light_sensor` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::LightSensor` rather than resolving the plugin on every call. iOS, macOS, and desktop have no supported public ambient-light API and return `Unavailable`; display brightness is never substituted for illumination.
