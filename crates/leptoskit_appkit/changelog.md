# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.1](https://github.com/ifiokjr/leptoskit/releases/tag/v0.1.1) (2026-11-01)

### Fixes

- **Expose `role="heading"` to `VoiceOver` as `AXHeading` without linking `NSAccessibilityHeadingRole`, which only exists from macOS 26, so macOS apps keep launching on macOS 14 through 25.** _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #185](https://github.com/ifiokjr/leptoskit/pull/185)
- **Measure macOS button titles with their native control geometry and display backing grid.** Keep custom button chrome consistent between layout and rendering. Preserve existing custom text measurers, explicit caller constraints and native font sizes. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #180](https://github.com/ifiokjr/leptoskit/pull/180)
