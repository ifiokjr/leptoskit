---
leptoskit_plugin_application: breaking
---

# leptoskit_plugin_application: app identity

`leptoskit_plugin_application` is the leptoskit plugin for installed host application identity: ID, localized display name, release version, and build number.

Add `leptoskit_plugin_application` as a direct dependency and select the `application` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::Application` rather than resolving the plugin on every call. Apple reads `Bundle.main`, Android reads the running package record, and a Rust desktop host supplies its own packaging metadata; browsers have no equivalent installed-app identity.
