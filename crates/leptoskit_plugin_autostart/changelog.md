# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.0](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.0) (2026-11-06)

### Breaking changes

#### leptoskit_plugin_autostart: login startup

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_autostart` is the leptoskit plugin for explicit per-user desktop login registration with a validated identity and command: a LaunchAgent on macOS, an XDG autostart entry on Linux, and the current user's Run key on Windows.

The native host constructs `Autostart` itself with the app's identity, executable, and arguments; it has no `native.plugins` entry and installs nothing by default. The host calls `enable`, `disable`, or `is_registered` directly, or registers it on the bridge so a guest with the `guest` feature can use `guest::AutostartClient` without choosing the command. Constructing it never enables login startup, an existing entry it doesn't own is never overwritten, and iOS, Android, and the browser return `Unavailable`.

### Fixes

- **Correct README claims about generated renderers, accessibility, runtime plugin registration, password screens, RPC macros, and interpreter updates.** Share crate summaries with Rust documentation and derive MSRV badges from Cargo through MDT templates, with a documentation drift check. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #515](https://github.com/leptoskit/leptoskit/pull/515)
