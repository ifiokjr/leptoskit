---
leptoskit_plugin_store: major
---

# leptoskit_plugin_store: JSON settings

`leptoskit_plugin_store` is the leptoskit plugin for bounded JSON application settings with explicit persistence and atomic ordered batches.

Add `leptoskit_plugin_store` as a direct dependency and select the `store` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::Store` rather than resolving the plugin on every call. Each application gets one store and guests cannot choose filesystem paths: native apps save to a file in the app data directory and browser apps use `web::Store` over `localStorage`. Nothing saves until `save` or a committed batch, and the store is not encrypted, so keep credentials in the secure store.
