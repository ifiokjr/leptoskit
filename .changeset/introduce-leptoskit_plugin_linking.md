---
leptoskit_plugin_linking: breaking
---

# leptoskit_plugin_linking: external links and settings

`leptoskit_plugin_linking` is the leptoskit plugin for validated external HTTP(S), email, phone, and SMS links plus application settings, without executing shell text.

Add `leptoskit_plugin_linking` as a direct dependency and select the `linking` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::Linking` rather than resolving the plugin on every call. Incoming deep links need host lifecycle integration and are not handled by this crate.
