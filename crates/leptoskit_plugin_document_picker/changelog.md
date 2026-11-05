# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.2.0](https://github.com/leptoskit/leptoskit/releases/tag/v0.2.0) (2026-11-05)

### Breaking changes

#### leptoskit_plugin_document_picker: document import

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_document_picker` is the leptoskit plugin for bounded document import through the iOS document picker, macOS open panel, Android Storage Access Framework, or an HTML file input.

Add `leptoskit_plugin_document_picker` as a direct dependency and select the `document-picker` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and call `pick`, browser apps call `web::pick` from a user action, and Rust desktop hosts on macOS, Windows, and Linux can call `desktop::pick` with a parent window. The user picks one document; the app receives its name, its MIME type when known, and owned bytes up to 4 MiB, and never gets a reusable path into the document provider.

### Fixes

- **Correct README claims about generated renderers, accessibility, runtime plugin registration, password screens, RPC macros, and interpreter updates.** Share crate summaries with Rust documentation and derive MSRV badges from Cargo through MDT templates, with a documentation drift check. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #515](https://github.com/leptoskit/leptoskit/pull/515)
