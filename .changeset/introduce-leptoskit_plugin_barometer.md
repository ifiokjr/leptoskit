---
leptoskit_plugin_barometer: major
---

# leptoskit_plugin_barometer: barometric pressure

`leptoskit_plugin_barometer` is the leptoskit plugin for cancellable atmospheric pressure readings in hectopascals.

Add `leptoskit_plugin_barometer` as a direct dependency and select the `barometer` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::Barometer` rather than resolving the plugin on every call. iOS and Android use real pressure sensors (iOS needs an `NSMotionUsageDescription`); desktop and the browser report the sensor as unavailable, and no relative altitude estimate is fabricated.
