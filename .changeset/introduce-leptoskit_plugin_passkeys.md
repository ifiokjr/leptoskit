---
leptoskit_plugin_passkeys: major
---

# leptoskit_plugin_passkeys: passkeys and Sign in with Apple

`leptoskit_plugin_passkeys` is the leptoskit plugin for passkeys and Sign in with Apple through the platform's own credential sheet. The sign-in client in `leptoskit_auth` uses it, so apps rarely call it directly.

Add `leptoskit_plugin_passkeys` as a direct dependency and select the `passkeys` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::Passkeys` rather than resolving the plugin on every call. Apple platforms use `ASAuthorization` and Android uses Credential Manager, with WebAuthn JSON crossing the bridge unchanged; Sign in with Apple is Apple-only. Windows and Linux report the calls unavailable, and the sign-in client falls back to the system browser there. The relying party's site must list the app in `apple-app-site-association` and `assetlinks.json`, which `leptoskit::server::app_associations` serves from `[native]`.
