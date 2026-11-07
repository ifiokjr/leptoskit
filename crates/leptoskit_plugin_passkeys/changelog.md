# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.1](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.1) (2026-11-07)

### Changed

- **No package-specific changes were recorded; `leptoskit_plugin_passkeys` was updated to 0.3.1 as part of group `release`.**

## [0.3.0](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.0) (2026-11-06)

### Breaking changes

#### leptoskit_plugin_passkeys: passkeys and Sign in with Apple

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_passkeys` is the leptoskit plugin for passkeys and Sign in with Apple through the platform's own credential sheet. The sign-in client in `leptoskit_auth` uses it, so apps rarely call it directly.

Add `leptoskit_plugin_passkeys` as a direct dependency and select the `passkeys` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::Passkeys` rather than resolving the plugin on every call. Apple platforms use `ASAuthorization` and Android uses Credential Manager, with WebAuthn JSON crossing the bridge unchanged; Sign in with Apple is Apple-only. Windows and Linux report the calls unavailable, and the sign-in client falls back to the system browser there. The relying party's site must list the app in `apple-app-site-association` and `assetlinks.json`, which `leptoskit::server::app_associations` serves from `[native]`.

### Fixes

- **Correct README claims about generated renderers, accessibility, runtime plugin registration, password screens, RPC macros, and interpreter updates.** Share crate summaries with Rust documentation and derive MSRV badges from Cargo through MDT templates, with a documentation drift check. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #515](https://github.com/leptoskit/leptoskit/pull/515)
