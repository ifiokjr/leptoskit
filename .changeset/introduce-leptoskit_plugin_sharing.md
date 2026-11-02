---
leptoskit_plugin_sharing: breaking
---

# leptoskit_plugin_sharing: native sharing

`leptoskit_plugin_sharing` is the leptoskit plugin for bounded text and HTTP(S) URL sharing through the user-selected native share UI.

Add `leptoskit_plugin_sharing` as a direct dependency and select the `sharing` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. The app supplies the content, the user picks the destination, and completion only means the platform finished its API callback.
