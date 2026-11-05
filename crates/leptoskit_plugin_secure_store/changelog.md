# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.2.0](https://github.com/ifiokjr/leptoskit/releases/tag/v0.2.0) (2026-11-05)

### Breaking changes

#### leptoskit_plugin_secure_store: credential-store secrets

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/ifiokjr/leptoskit/pull/429)

`leptoskit_plugin_secure_store` is the leptoskit plugin for small binary secrets, such as a refresh token, kept in the platform credential store and scoped to one application.

Add `leptoskit_plugin_secure_store` as a direct dependency and select the `secure-store` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::SecureStore` to get, set, and delete secrets of up to 2048 bytes; all of an app's secrets share one encrypted record limited to 2560 bytes and 64 entries, which hosts can lower. iOS and macOS use the Keychain and Android encrypts with an Android Keystore key, while desktop Rust hosts construct `SecureStore` over the Keychain, Windows Credential Manager, or Secret Service. The browser has no secure store, and there is no key listing, biometric prompt, or cloud sync.

### Fixes

- **Correct README claims about generated renderers, accessibility, runtime plugin registration, password screens, RPC macros, and interpreter updates.** Share crate summaries with Rust documentation and derive MSRV badges from Cargo through MDT templates, with a documentation drift check. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #515](https://github.com/ifiokjr/leptoskit/pull/515)
