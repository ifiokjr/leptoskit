# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.2.0](https://github.com/leptoskit/leptoskit/releases/tag/v0.2.0) (2026-11-05)

### Breaking changes

#### leptoskit_plugin_barometer: barometric pressure

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_barometer` is the leptoskit plugin for cancellable atmospheric pressure readings in hectopascals.

Add `leptoskit_plugin_barometer` as a direct dependency and select the `barometer` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::Barometer` rather than resolving the plugin on every call. iOS and Android use real pressure sensors (iOS needs an `NSMotionUsageDescription`); desktop and the browser report the sensor as unavailable, and no relative altitude estimate is fabricated.

### Fixes

- **Correct README claims about generated renderers, accessibility, runtime plugin registration, password screens, RPC macros, and interpreter updates.** Share crate summaries with Rust documentation and derive MSRV badges from Cargo through MDT templates, with a documentation drift check. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #515](https://github.com/leptoskit/leptoskit/pull/515)
