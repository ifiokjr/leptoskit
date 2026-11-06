# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.0](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.0) (2026-11-06)

### Breaking changes

#### leptoskit_plugin_dialog: native dialogs

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_dialog` is the leptoskit library for asynchronous native file, directory, save, and message dialogs on macOS, Windows, and Linux, plus browser file selection.

Rust desktop hosts and browser apps call it directly: it has no `native.plugins` entry, no guest API, and no automatic registration, because each dialog needs a running event loop and a live parent window. On desktop, `desktop::FileDialog` and `desktop::MessageDialog` borrow that parent window until the dialog closes, and Linux needs an XDG desktop portal and Zenity. In the browser, `web::open_files` uses the File System Access API from a user action on HTTPS, and `web::read_file` reads a selection under an explicit byte limit; browser save, directory, and message dialogs, and mobile dialogs, are not provided.

### Fixes

- **Correct README claims about generated renderers, accessibility, runtime plugin registration, password screens, RPC macros, and interpreter updates.** Share crate summaries with Rust documentation and derive MSRV badges from Cargo through MDT templates, with a documentation drift check. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #515](https://github.com/leptoskit/leptoskit/pull/515)
