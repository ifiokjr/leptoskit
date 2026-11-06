# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.0](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.0) (2026-11-06)

### Breaking changes

#### leptoskit_testing: native widget tests

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_testing` runs Flutter-style widget tests over the production renderer and Yoga layout, so a screen can be tested without a device or simulator.

`WidgetTester` renders a screen, finds elements by text, label, role, or test id with Testing Library-style queries (`get_by_text`, `query_by_label`, and the rest), acts on them, and pumps the renderer a bounded number of times while waiting for a result. The `native-ui` feature goes further and drives the real app through each platform's own automation: XCUITest on iOS, UI Automator on Android, XCTest on macOS, UI Automation on Windows, and AT-SPI on Linux.

### Fixes

- **Correct README claims about generated renderers, accessibility, runtime plugin registration, password screens, RPC macros, and interpreter updates.** Share crate summaries with Rust documentation and derive MSRV badges from Cargo through MDT templates, with a documentation drift check. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #515](https://github.com/leptoskit/leptoskit/pull/515)
