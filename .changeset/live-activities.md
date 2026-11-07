---
"leptoskit_plugin_live_activities": minor
---

# iOS Live Activities

Apps can start, update, and end iOS Live Activities (Lock Screen and Dynamic Island) through the new `leptoskit_plugin_live_activities` crate and the `live-activities` runtime plugin. Guests call `are_enabled`, `start`, `update`, `end`, `wait_push_token`, and `list`. Content state is a bounded string map that a Widget Extension renders via `LeptosKitLiveActivityAttributes`.

Only iOS implements ActivityKit. Android, macOS, the browser, and the unsupported host stub return `Unavailable` after validating payloads. Apps must set `NSSupportsLiveActivities` in Info.plist, ship a Widget Extension, and enable Push Notifications when requesting push tokens. Custom attribute types, push-to-start tokens, and remote payload helpers are still out of scope.
