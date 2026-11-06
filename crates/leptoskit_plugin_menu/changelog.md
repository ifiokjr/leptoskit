# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.2.0](https://github.com/leptoskit/leptoskit/releases/tag/v0.2.0) (2026-11-05)

### Breaking changes

#### leptoskit_plugin_menu: native menus

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_menu` is the native-host plugin for bounded native menu definitions with explicit desktop ownership: commands, checkboxes, separators, submenus, and accelerators such as `CmdOrCtrl+S`.

Native hosts construct it explicitly on their UI thread; it is not a guest plugin and installs nothing by default. macOS attaches it as the application menu and Windows attaches it to a window the host owns; Linux has no standalone menu bar, so there it only feeds tray menus. Definitions are validated before any native allocation, and iOS, Android, and the browser only validate definitions and show no menu.

### Fixes

- **Correct README claims about generated renderers, accessibility, runtime plugin registration, password screens, RPC macros, and interpreter updates.** Share crate summaries with Rust documentation and derive MSRV badges from Cargo through MDT templates, with a documentation drift check. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #515](https://github.com/leptoskit/leptoskit/pull/515)
