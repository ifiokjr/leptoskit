# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.0](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.0) (2026-11-06)

### Breaking changes

#### leptoskit_plugin_pedometer: step counts

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_pedometer` is the leptoskit plugin for bounded, demand-driven step-counter readings with explicit platform count domains.

Add `leptoskit_plugin_pedometer` as a direct dependency and select the `pedometer` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::Pedometer` rather than resolving the plugin on every call. Only iOS (CoreMotion) and Android (its step counter) count steps; the app supplies the motion usage description or activity-recognition permission. iOS reports steps for the requested interval and can read history from the past seven days, while Android reports its cumulative count since boot, so neither is a daily total. macOS, Windows, Linux, and the browser report no sensor and never invent counts.

### Fixes

- **Correct README claims about generated renderers, accessibility, runtime plugin registration, password screens, RPC macros, and interpreter updates.** Share crate summaries with Rust documentation and derive MSRV badges from Cargo through MDT templates, with a documentation drift check. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #515](https://github.com/leptoskit/leptoskit/pull/515)
