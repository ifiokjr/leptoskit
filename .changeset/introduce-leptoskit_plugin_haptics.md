---
leptoskit_plugin_haptics: breaking
---

# leptoskit_plugin_haptics: haptic feedback

`leptoskit_plugin_haptics` is the leptoskit plugin for selection, impact, and notification feedback on native actuators.

Add `leptoskit_plugin_haptics` as a direct dependency and select the `haptics` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::Haptics` rather than resolving the plugin on every call. iOS uses UIKit feedback generators, Android uses platform haptic feedback, and macOS approximates the mobile styles with its trackpad performer; browsers use the Vibration API where allowed.
