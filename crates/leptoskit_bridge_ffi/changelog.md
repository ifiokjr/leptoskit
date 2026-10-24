# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.1](https://github.com/ifiokjr/leptoskit/releases/tag/v0.1.1) (2026-10-24)

### Features

- **Retain Android service wake callbacks across native threads, poll application services on the main looper, and join service shutdown before application replacement, recovery, or unload completes.** _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #168](https://github.com/ifiokjr/leptoskit/pull/168)

### Fixes

- **Remove Android Button's native padding before applying shared layout frames so compact buttons paint their labels.** _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #172](https://github.com/ifiokjr/leptoskit/pull/172)
- **Separate Android service scheduling, native request jobs, widget rendering, and widget properties from application lifecycle control while preserving native callback and shutdown ordering.** _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #168](https://github.com/ifiokjr/leptoskit/pull/168)
- **Complete joined guest retirement after cleanup errors, preserve confirmed replacements, and retain recovery diagnostics through fallback.** Guard delayed recovery errors against stale application generations. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #168](https://github.com/ifiokjr/leptoskit/pull/168)
- **Use the existing AppKit font measurer for macOS application layout so native multiline Text receives the space its unchanged native renderer requires.** _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #172](https://github.com/ifiokjr/leptoskit/pull/172)
