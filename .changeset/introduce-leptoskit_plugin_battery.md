---
leptoskit_plugin_battery: breaking
---

# leptoskit_plugin_battery: battery and power state

`leptoskit_plugin_battery` is the leptoskit plugin for battery charge, charging state, and system power-saving snapshots with cancellable change observation.

Add `leptoskit_plugin_battery` as a direct dependency and select the `battery` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::Battery` rather than resolving the plugin on every call. Unknown readings stay unknown instead of being reported as a full battery, and a simulator is never presented as physical hardware.
