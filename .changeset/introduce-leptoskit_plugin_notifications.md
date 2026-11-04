---
leptoskit_plugin_notifications: major
---

# leptoskit_plugin_notifications: local notifications

`leptoskit_plugin_notifications` is the leptoskit plugin for immediate local system notifications with explicit permission requests, bounded messages, and cancellation.

Add `leptoskit_plugin_notifications` as a direct dependency and select the `notifications` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::Notifications` rather than resolving the plugin on every call. iOS, macOS, and Android post through the system (Android also configures channels), while browser and Linux apps call the `web` and `linux` modules directly; Windows has no backend yet. A successful call means the OS accepted the request, not that a banner appeared, and scheduled notifications, push, and tap routing are not implemented.
