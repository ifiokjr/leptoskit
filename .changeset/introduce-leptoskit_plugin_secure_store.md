---
leptoskit_plugin_secure_store: breaking
---

# leptoskit_plugin_secure_store: credential-store secrets

`leptoskit_plugin_secure_store` is the leptoskit plugin for small binary secrets in the platform credential store, scoped to one application.

Add `leptoskit_plugin_secure_store` as a direct dependency and select the `secure-store` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::SecureStore` rather than resolving the plugin on every call. Native mobile handlers ship with the crate; desktop hosts construct the store over their own credential backend, and secrets never become a guest-selected filesystem path.
