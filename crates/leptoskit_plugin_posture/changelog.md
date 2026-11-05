# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.2.0](https://github.com/leptoskit/leptoskit/releases/tag/v0.2.0) (2026-11-05)

### Breaking changes

#### leptoskit_plugin_posture: foldable posture

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_posture` is the leptoskit plugin for fold state, hinge geometry, and hinge angles, so a layout can put content on either side of a fold.

Add `leptoskit_plugin_posture` as a direct dependency and select the `posture` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::DevicePosture` rather than resolving the plugin on every call, and browser apps use the `web` module. iOS 27.1 uses the hinge interaction API, Android uses Jetpack WindowManager and the hinge angle sensor, HarmonyOS (experimental) uses its display fold status, and Chromium browsers report the fold without angles. macOS, Windows, and Linux report the plugin unavailable.

### Fixes

- **Correct README claims about generated renderers, accessibility, runtime plugin registration, password screens, RPC macros, and interpreter updates.** Share crate summaries with Rust documentation and derive MSRV badges from Cargo through MDT templates, with a documentation drift check. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #515](https://github.com/leptoskit/leptoskit/pull/515)
