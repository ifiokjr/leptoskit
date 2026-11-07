# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.1](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.1) (2026-11-07)

### Changed

- **No package-specific changes were recorded; `leptoskit_plugin_light_sensor` was updated to 0.3.1 as part of group `release`.**

## [0.3.0](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.0) (2026-11-06)

### Breaking changes

#### leptoskit_plugin_light_sensor: ambient light

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_light_sensor` is the leptoskit plugin for ambient illuminance in lux from Android's light sensor or the browser's AmbientLightSensor.

Add `leptoskit_plugin_light_sensor` as a direct dependency and select the `light_sensor` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::LightSensor` rather than resolving the plugin on every call. iOS, macOS, and desktop have no supported public ambient-light API and return `Unavailable`; display brightness is never substituted for illumination.

### Fixes

- **Correct README claims about generated renderers, accessibility, runtime plugin registration, password screens, RPC macros, and interpreter updates.** Share crate summaries with Rust documentation and derive MSRV badges from Cargo through MDT templates, with a documentation drift check. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #515](https://github.com/leptoskit/leptoskit/pull/515)
