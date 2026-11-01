# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.1](https://github.com/ifiokjr/leptoskit/releases/tag/v0.1.1) (2026-11-01)

### Fixes

- **Prevent overlapping owners from corrupting installed updates.** Hold an exclusive file lock for the lifetime of each installed-update store. Overlapping opens, including during iOS controller replacement, return a diagnostic before reading or changing selector state. Dropping or destroying the previous owner releases the store for a retry. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #242](https://github.com/ifiokjr/leptoskit/pull/242)
