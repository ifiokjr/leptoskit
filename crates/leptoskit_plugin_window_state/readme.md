<h1 align="center">leptoskit_plugin_window_state</h1>

<p align="center">
  <strong>leptoskit_plugin_window_state</strong> is the native-host plugin for validated window placement and explicit native window restoration.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://docs.rs/leptoskit_plugin_window_state"><strong>Docs</strong></a> ·
  <a href="https://github.com/ifiokjr/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<p align="center">
  <a href="https://crates.io/crates/leptoskit_plugin_window_state"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_plugin_window_state.svg"></a>
  <a href="https://docs.rs/leptoskit_plugin_window_state"><img alt="docs.rs" src="https://docs.rs/leptoskit_plugin_window_state/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<br>

## Overview

Window placement and restoration inspired by [Tauri window-state](https://v2.tauri.app/plugin/window-state/) and [Electron window bounds](https://www.electronjs.org/docs/latest/api/browser-window).

Persist `WindowState` through your application's storage, then call `placement` with current `Display` work areas. Deserialization validates geometry. Disconnected monitors, oversized windows, negative monitor coordinates, and changed work areas are handled without restoring an inaccessible window. Coordinates are logical units; the host supplies all rectangles in one consistent coordinate system.

On macOS, `macos::capture` and `macos::restore` operate on the exact `NSWindow` supplied by its owner. Track normal bounds before maximizing; fullscreen needs the host's asynchronous window lifecycle and returns `Unavailable`. Restoration does not activate the application.

The portable state/placement API works on every target. Native capture/restore adapters for Windows, Linux, browsers, and mobile are not implemented. There is no automatic global-window lookup, guest capability, or implicit persistence. A host deliberately controls which window is restored and when its state is saved.

## How it fits

Native hosts construct it explicitly; it is not a guest plugin and installs nothing by default. The portable state and placement API works everywhere; native capture and restore adapters currently ship for macOS, and the host owns persistence and which window is restored.

## Use it

```toml
# Host-only: construct it explicitly; there is no `native.plugins` entry.
[dependencies]
leptoskit_plugin_window_state = "0.1"
```

## Learn more

- [Crate documentation](https://docs.rs/leptoskit_plugin_window_state)
- [leptoskit documentation](https://ifiokjr.github.io/leptoskit/)
- [Repository](https://github.com/ifiokjr/leptoskit)
- [Plugin inventory](https://ifiokjr.github.io/leptoskit/platforms/plugin-inventory.html)
