# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.2.0](https://github.com/ifiokjr/leptoskit/releases/tag/v0.2.0) (2026-11-05)

### Breaking changes

#### leptoskit_rive: Rive playback

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/ifiokjr/leptoskit/pull/429)

`leptoskit_rive` plays Rive animations that ship inside the app: it checks a `.riv` file before loading it, picks one animation or state machine to run, and sends typed state-machine inputs (booleans, numbers, and triggers) so an animation can react to the interface.

The shared `Rive` component uses it on the web through the `web` feature and on macOS, iOS, Android, Linux, and Windows through the `native` feature; the terminal has no Rive playback. Native playback draws vector scenes only and rejects files that use images, fonts, text, layout, scripts, or audio. It never fetches anything over the network, and it should only load files you trust.

### Fixes

- **Correct README claims about generated renderers, accessibility, runtime plugin registration, password screens, RPC macros, and interpreter updates.** Share crate summaries with Rust documentation and derive MSRV badges from Cargo through MDT templates, with a documentation drift check. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #515](https://github.com/ifiokjr/leptoskit/pull/515)
