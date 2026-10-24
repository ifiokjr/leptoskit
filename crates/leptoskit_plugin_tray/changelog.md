# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.1](https://github.com/ifiokjr/leptoskit/releases/tag/v0.1.1) (2026-10-24)

### Features

#### Add host-owned desktop menus and tray icons

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #66](https://github.com/ifiokjr/leptoskit/pull/66)

Add independent `menu` and `tray` crates built on muda and tray-icon, without a Tauri application. Menu definitions are bounded and validated before native allocation. macOS hosts attach an explicit application-menu lease and Windows hosts an HWND menu lease; dropping a lease restores the previous menu. Activations flow through a bounded event channel that the host installs explicitly.

The tray crate owns up to 16 icons on macOS/Windows and one KSNI icon on Linux, with validated RGBA images, optional menus, coalesced bounded events and ordered teardown. Linux deliberately avoids GTK; standalone Linux menu bars, standard roles, context menus and guest registration are not provided.
