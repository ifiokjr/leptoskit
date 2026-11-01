---
leptoskit_plugin_filesystem: breaking
---

# leptoskit_plugin_filesystem: application files

`leptoskit_plugin_filesystem` is the leptoskit plugin for application-private binary files on native and OPFS in the browser, with bounded messages.

Add `leptoskit_plugin_filesystem` as a direct dependency and select the `filesystem` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::Filesystem` rather than resolving the plugin on every call. Every native path stays inside the application-owned data directory; guests cannot select an arbitrary filesystem root.
