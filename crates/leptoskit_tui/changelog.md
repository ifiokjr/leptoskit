# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.2.0](https://github.com/leptoskit/leptoskit/releases/tag/v0.2.0) (2026-11-05)

### Breaking changes

#### leptoskit_tui: the terminal backend

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_tui` draws the same shared element tree in the terminal, for long-lived interactive apps such as editors, dashboards, and chat clients, with keyboard focus, the cursor, mouse input, and scrolling handled by the backend.

Every cell is 8×16 logical pixels, so Yoga layout, Tailwind spacing, and text measurement land on the monospace grid. `leptoskit dev --platform tui` and `leptoskit build --platform tui` run it, and `examples/opencode` is a full coding-agent interface built with it. The terminal's font is the user's, so font sizes and weights become attributes and colors; `Image` and `Vector` draw a placeholder, Rive playback is unsupported, and right-to-left text is not reordered.

### Fixes

- **Correct README claims about generated renderers, accessibility, runtime plugin registration, password screens, RPC macros, and interpreter updates.** Share crate summaries with Rust documentation and derive MSRV badges from Cargo through MDT templates, with a documentation drift check. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #515](https://github.com/leptoskit/leptoskit/pull/515)
