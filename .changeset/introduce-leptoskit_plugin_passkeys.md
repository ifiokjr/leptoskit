---
leptoskit_plugin_passkeys: breaking
---

# leptoskit_plugin_passkeys: passkeys and Sign in with Apple

`leptoskit_plugin_passkeys` is the leptoskit plugin for passkeys and Sign in with Apple through the platform's own credential sheet.

Add `leptoskit_plugin_passkeys` as a direct dependency and select the `passkeys` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::Passkeys` rather than resolving the plugin on every call. Apple platforms use `ASAuthorization` and Android uses Credential Manager; WebAuthn JSON crosses the bridge unchanged.
