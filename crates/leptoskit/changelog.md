# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.0](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.0) (2026-11-06)

### Breaking changes

#### leptoskit: the one dependency

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit` is the crate an app depends on. It re-exports the components, Tailwind styles, layout engine, renderer, and Wasm runtime, with one feature per optional part, so an app names `leptoskit` alone and never keeps the versions of its parts in step by hand.

Pick how the interface runs: `web` with `csr`, `hydrate`, `ssr`, or `islands` renders through leptos, and a backend feature (`macos`, `ios`, `android`, `linux`, `windows`, or `tui`) commits the same tree as real widgets or as terminal cells. `server` adds the Axum toolkit, and `analytics` beside it keeps request and browser measurements. Everything else is opt-in: `rpc`, `query`, `form`, `aria`, `table`, `virtual`, `l10n`, `rive`, `db` with `db-postgres` or `db-sqlite`, `auth` (passwords, passkeys, and providers through their own `auth-*` features), `idp`, `extension`, `codepush`, and `devtools`. `leptoskit::secrets` reads values that Monosecret, the default secret manager, or a trusted host supplies.

### Features

- **Theme tokens in leptoskit.toml and deeper CSS var().** `[theme.colors]` and `[theme.lengths]` in `leptoskit.toml` name design tokens that `Theme::install_maps` loads into the process-wide Theme map. Arbitrary color and length utilities accept `var(--name)`, one nested `var()` fallback, and `/opacity` on color vars. The web adapter prepends `Theme::css()` so the browser sees the same `:root` custom properties native resolves through Theme. Tailwind `@theme` / `tailwind.config` files remain unsupported. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #538](https://github.com/leptoskit/leptoskit/pull/538)

### Fixes

- **Correct README claims about generated renderers, accessibility, runtime plugin registration, password screens, RPC macros, and interpreter updates.** Share crate summaries with Rust documentation and derive MSRV badges from Cargo through MDT templates, with a documentation drift check. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #515](https://github.com/leptoskit/leptoskit/pull/515)
