# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.1](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.1) (2026-11-07)

### Changed

- **No package-specific changes were recorded; `leptoskit_gtk` was updated to 0.3.1 as part of group `release`.**

## [0.3.0](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.0) (2026-11-06)

### Breaking changes

#### leptoskit_gtk: the Linux backend

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_gtk` renders the shared element tree as real GTK 4 Linux widgets, consuming the same Yoga mutation stream as every other backend, and draws every built-in component. It needs GTK 4.6 or newer.

The Linux app `leptoskit new` generates uses it through the `linux` feature. A host initializes GTK on the application thread, hands a `GtkBackend` to `NativeSurface`, shows a committed root with `open_window`, and runs the loop with `run_application`; GTK objects stay on their creating thread.

### Fixes

- **GTK grid parity probe against Yoga frames.** Adds a `grid_parity_gtk` example that mounts the same Tailwind grid/flex fixtures as the web probe on real GTK 4 widgets under Xvfb, asserts allocations match Yoga within half a CSS pixel, and writes PNGs for cross-device layout evidence. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #542](https://github.com/leptoskit/leptoskit/pull/542)
- **Correct README claims about generated renderers, accessibility, runtime plugin registration, password screens, RPC macros, and interpreter updates.** Share crate summaries with Rust documentation and derive MSRV badges from Cargo through MDT templates, with a documentation drift check. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #515](https://github.com/leptoskit/leptoskit/pull/515)
- **Paint italic text and per-edge borders on Linux and Android.** `border-t`, `border-l-2`, and the other edge border utilities now get the same default stroke color as a uniform `border`, and GTK and Android paint each edge instead of leaving those strokes undrawn. The `italic` utility also draws through GTK labels and Android text views, matching the web and the terminal. AppKit, UIKit, and Win32 still leave per-edge borders undrawn and italic upright. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #534](https://github.com/leptoskit/leptoskit/pull/534)
