# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.2.0](https://github.com/leptoskit/leptoskit/releases/tag/v0.2.0) (2026-11-05)

### Breaking changes

#### leptoskit_rpc_macros: RPC attributes

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_rpc_macros` provides the attributes behind `leptoskit_rpc`: `#[query]`, `#[mutation]`, and `#[action]` declare operations; `#[stream]`, `#[channel]`, `#[job]`, and `#[cron]` declare live streams, channels, background jobs, and scheduled jobs; and `#[api]` publishes a leptos server function to generated clients.

Use them through `leptoskit_rpc`, which re-exports them. Every target gets the operation's typed argument struct so any client can call it; only a server build keeps the function body and registers its route.

### Fixes

- **Correct README claims about generated renderers, accessibility, runtime plugin registration, password screens, RPC macros, and interpreter updates.** Share crate summaries with Rust documentation and derive MSRV badges from Cargo through MDT templates, with a documentation drift check. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #515](https://github.com/leptoskit/leptoskit/pull/515)
