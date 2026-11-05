# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.2.0](https://github.com/ifiokjr/leptoskit/releases/tag/v0.2.0) (2026-11-05)

### Breaking changes

#### leptoskit_analytics: see which routes fail or slow down

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #513](https://github.com/ifiokjr/leptoskit/pull/513)

`leptoskit_analytics` keeps bounded, in-memory request and browser measurements for a leptoskit app, so you can see which routes fail or slow down without running a separate observability service.

Enable `analytics` alongside `server` on the `leptoskit` dependency. The `server` feature instruments Axum, continues W3C `traceparent` correlation, and serves a protected JSON evidence API at `/_leptoskit/analytics` plus a same-origin browser module at `/_leptoskit/analytics.mjs`. The browser module honors Global Privacy Control and Do Not Track, and collects no user or session ID, cookie, query string, or page content. Each replica reports its own snapshot; connect the deployment service (`lk console serve`) for durable fleet history, or export OTLP/HTTP metrics.

### Fixes

- **Correct README claims about generated renderers, accessibility, runtime plugin registration, password screens, RPC macros, and interpreter updates.** Share crate summaries with Rust documentation and derive MSRV badges from Cargo through MDT templates, with a documentation drift check. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #515](https://github.com/ifiokjr/leptoskit/pull/515)
