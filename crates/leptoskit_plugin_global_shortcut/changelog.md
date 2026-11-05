# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.2.0](https://github.com/leptoskit/leptoskit/releases/tag/v0.2.0) (2026-11-05)

### Breaking changes

#### leptoskit_plugin_global_shortcut: global shortcuts

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_global_shortcut` is the leptoskit plugin for scoped desktop global shortcuts with explicit registration conflicts and bounded events.

The native host creates `GlobalShortcuts` on its event-loop thread after startup; it has no `native.plugins` entry and installs nothing by default. `register` returns a guard that unregisters when dropped, and a host can wrap the manager in `ShortcutPlugin` so a guest with the `guest` feature registers shortcuts and polls events through `guest::GlobalShortcut`. macOS, Windows, and X11 are supported; native Wayland, browsers, and mobile return `Unavailable`, with no XWayland fallback or fake browser shortcut.

### Fixes

- **Correct README claims about generated renderers, accessibility, runtime plugin registration, password screens, RPC macros, and interpreter updates.** Share crate summaries with Rust documentation and derive MSRV badges from Cargo through MDT templates, with a documentation drift check. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #515](https://github.com/leptoskit/leptoskit/pull/515)
