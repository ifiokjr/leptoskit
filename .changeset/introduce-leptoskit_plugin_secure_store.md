---
leptoskit_plugin_secure_store: major
---

# leptoskit_plugin_secure_store: credential-store secrets

`leptoskit_plugin_secure_store` is the leptoskit plugin for small binary secrets, such as a refresh token, kept in the platform credential store and scoped to one application.

Add `leptoskit_plugin_secure_store` as a direct dependency and select the `secure-store` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::SecureStore` to get, set, and delete secrets of up to 2048 bytes; all of an app's secrets share one encrypted record limited to 2560 bytes and 64 entries, which hosts can lower. iOS and macOS use the Keychain and Android encrypts with an Android Keystore key, while desktop Rust hosts construct `SecureStore` over the Keychain, Windows Credential Manager, or Secret Service. The browser has no secure store, and there is no key listing, biometric prompt, or cloud sync.
