# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.1](https://github.com/ifiokjr/leptoskit/releases/tag/v0.1.1) (2026-11-01)

### Fixes

- **Keep desktop plugin helpers out of Wasm builds.** Scope web-auth's loopback timing constants to its desktop implementation and compile the WebSocket browser test server's connection helpers only on native hosts, so checking every target for Wasm succeeds. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #479](https://github.com/ifiokjr/leptoskit/pull/479)
