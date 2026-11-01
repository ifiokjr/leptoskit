# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.1](https://github.com/ifiokjr/leptoskit/releases/tag/v0.1.1) (2026-11-01)

### Fixes

- **Keep generated live lists and pages in sync.** Apply page row patches and cursor metadata together in TypeScript, Python, Dart, Rust and Go clients. Inserting a row before another row now preserves both. Generate valid Dart types for nullable model fields such as page cursors. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #473](https://github.com/ifiokjr/leptoskit/pull/473)
