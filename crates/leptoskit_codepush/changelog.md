# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.0](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.0) (2026-11-06)

### Breaking changes

#### leptoskit_codepush: over-the-air updates

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_codepush` ships interface changes to native apps without a store review: signed, content-hashed releases of the Wasm bundle, packed into one file, with binary deltas so a device downloads only the chunks that changed.

`leptoskit codepush` packs, signs, and publishes releases to a static channel layout that any file server, object store, or CDN can host, and `leptoskit_wasm` stages them on the device. A client accepts only releases signed by a trusted key and newer than what it runs, so a compromised registry can neither ship its own code nor roll a device back. A host can also require independent source approvals: rebuilders who built the same files from a public commit and signed to say so.

### Fixes

- **Correct README claims about generated renderers, accessibility, runtime plugin registration, password screens, RPC macros, and interpreter updates.** Share crate summaries with Rust documentation and derive MSRV badges from Cargo through MDT templates, with a documentation drift check. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #515](https://github.com/leptoskit/leptoskit/pull/515)
