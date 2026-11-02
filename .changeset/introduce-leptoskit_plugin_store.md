---
leptoskit_plugin_store: breaking
---

# leptoskit_plugin_store: JSON settings

`leptoskit_plugin_store` is the leptoskit plugin for bounded JSON application settings with explicit persistence and atomic ordered batches.

Add `leptoskit_plugin_store` as a direct dependency and select the `store` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::Store` rather than resolving the plugin on every call. Each application gets one store, values are JSON over a binary envelope, and guests cannot choose filesystem paths.
