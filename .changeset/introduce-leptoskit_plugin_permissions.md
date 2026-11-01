---
leptoskit_plugin_permissions: breaking
---

# leptoskit_plugin_permissions: permissions

`leptoskit_plugin_permissions` is the leptoskit plugin for explicit camera and microphone permission queries and requests.

Add `leptoskit_plugin_permissions` as a direct dependency and select the `permissions` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::Permissions` rather than resolving the plugin on every call. Querying status never shows a prompt; request only after the user chooses, and use the dedicated location, notification, and photo APIs for permissions with different OS flows.
