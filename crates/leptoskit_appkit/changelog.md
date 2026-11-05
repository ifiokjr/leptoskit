# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.2.0](https://github.com/ifiokjr/leptoskit/releases/tag/v0.2.0) (2026-11-05)

### Breaking changes

#### leptoskit_appkit: the macOS backend

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/ifiokjr/leptoskit/pull/429)

`leptoskit_appkit` is the macOS backend: it applies leptoskit mutation batches to real `NSView`s, from labels and buttons to `NSSecureTextField` password fields, sliders, and scroll views, and measures text with AppKit's own fonts.

A `NativeSurface` built on `AppKitBackend` renders the same tree the web build renders as DOM, and `snapshot_png` draws it offscreen without a window. AppKit is the reference backend that the others follow, and it draws every built-in component. The `macos` feature brings it in, but the macOS app `leptoskit new` generates still draws through a smaller Swift renderer, which covers ten components so far. On other platforms the crate compiles to an empty library.

### Fixes

- **Correct README claims about generated renderers, accessibility, runtime plugin registration, password screens, RPC macros, and interpreter updates.** Share crate summaries with Rust documentation and derive MSRV badges from Cargo through MDT templates, with a documentation drift check. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #515](https://github.com/ifiokjr/leptoskit/pull/515)
