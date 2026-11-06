# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.2.0](https://github.com/leptoskit/leptoskit/releases/tag/v0.2.0) (2026-11-05)

### Breaking changes

#### leptoskit_native: components, Tailwind, and the renderer

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_native` is the native side of the shared tree: builders for every component, from `View`, `Text`, and `Button` to `TextInput`, `PasswordInput`, `Slider`, and `ScrollView`, the Tailwind compiler, the retained element tree, and the diff that turns a commit into a mutation batch.

Every native backend implements `NativeBackend` and consumes the mutations this crate produces; the web build uses its optional `web` feature to render the same builders through leptos. App code normally reaches it through `leptoskit::prelude`, not as a direct dependency, so one screen works on every target.

### Fixes

- **Correct README claims about generated renderers, accessibility, runtime plugin registration, password screens, RPC macros, and interpreter updates.** Share crate summaries with Rust documentation and derive MSRV badges from Cargo through MDT templates, with a documentation drift check. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #515](https://github.com/leptoskit/leptoskit/pull/515)
