# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.2.0](https://github.com/leptoskit/leptoskit/releases/tag/v0.2.0) (2026-11-05)

### Breaking changes

#### leptoskit_query: async state in the TanStack style

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_query` is TanStack Query for leptos: screens that load data get caching, deduplication, retries, background refetching, infinite queries, and optimistic mutations, all exposed as fine-grained signals so a view re-renders only when the value it reads changes.

Declare a `QueryScope` or `MutationScope` and read it with `use_query` or `use_mutation`. Queries refetch on focus and reconnect, pause offline, stay in sync across browser tabs or native windows through `QueryClient::broadcast`, and can be fetched during server rendering with `with_ssr`. The default `browser` feature wires up the browser's timers and events; native guests enable `guest` instead, and the `db` feature refetches the queries a `leptoskit_db` live change touches. `leptoskit_rpc` builds its client calls on it.

### Fixes

- **Correct README claims about generated renderers, accessibility, runtime plugin registration, password screens, RPC macros, and interpreter updates.** Share crate summaries with Rust documentation and derive MSRV badges from Cargo through MDT templates, with a documentation drift check. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #515](https://github.com/leptoskit/leptoskit/pull/515)
