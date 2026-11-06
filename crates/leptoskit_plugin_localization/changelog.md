# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.0](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.0) (2026-11-06)

### Breaking changes

#### leptoskit_plugin_localization: locale preferences

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_localization` is the leptoskit plugin for ordered preferred locales, language and region codes, calendar, timezone, first weekday, and clock preference.

Add `leptoskit_plugin_localization` as a direct dependency and select the `localization` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::Localization` rather than resolving the plugin on every call. iOS, macOS, and Android read the system settings, desktop Rust hosts register `desktop::Localization`, and the browser calls `web::snapshot()`. Each snapshot reflects current settings and no global locale is mutated; there are no change events yet, so re-query after the app resumes.

### Fixes

- **Correct README claims about generated renderers, accessibility, runtime plugin registration, password screens, RPC macros, and interpreter updates.** Share crate summaries with Rust documentation and derive MSRV badges from Cargo through MDT templates, with a documentation drift check. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #515](https://github.com/leptoskit/leptoskit/pull/515)
