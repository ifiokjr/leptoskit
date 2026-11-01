# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.1](https://github.com/ifiokjr/leptoskit/releases/tag/v0.1.1) (2026-11-01)

### Fixes

- **Bound the wait for an unavailable read replica.** Include connection attempts and position queries in the replica replay wait, so a stalled standby falls back to the primary within the configured budget. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #481](https://github.com/ifiokjr/leptoskit/pull/481)
