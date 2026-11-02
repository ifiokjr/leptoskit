---
leptoskit_plugin_notifications: breaking
---

# leptoskit_plugin_notifications: local notifications

`leptoskit_plugin_notifications` is the leptoskit plugin for immediate local system notifications with explicit permission requests, bounded messages, and cancellation.

Add `leptoskit_plugin_notifications` as a direct dependency and select the `notifications` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::Notifications` rather than resolving the plugin on every call. A successful call means the OS accepted the request, not that a banner appeared; focus modes and user settings can still suppress it.
