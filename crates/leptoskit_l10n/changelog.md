# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.2.0](https://github.com/leptoskit/leptoskit/releases/tag/v0.2.0) (2026-11-05)

### Breaking changes

#### leptoskit_l10n: typed translations

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_l10n` provides typed translations in the style of Flutter's `gen_l10n`: locales, negotiation, and plural rules behind a generated messages module.

Messages live in one ARB file per locale; `leptoskit l10n` writes the typed module, and app code calls getters instead of parsing ARB at runtime. It works in SSR, the browser, and native guests.

### Fixes

- **Correct README claims about generated renderers, accessibility, runtime plugin registration, password screens, RPC macros, and interpreter updates.** Share crate summaries with Rust documentation and derive MSRV badges from Cargo through MDT templates, with a documentation drift check. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #515](https://github.com/leptoskit/leptoskit/pull/515)
