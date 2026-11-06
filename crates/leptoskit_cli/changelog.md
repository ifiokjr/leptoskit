# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.0](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.0) (2026-11-06)

### Breaking changes

#### leptoskit_cli: the command line

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_cli` is the `leptoskit` and `lk` command: scaffold a project with `new`, run every platform at once with `dev`, then `build`, `serve`, and `test`. It also writes routes, translations, and database migrations, generates sign-in screens and plugins, ships code push releases, uploads to the app and extension stores with `publish`, drives devices and simulators (also over MCP), and lints and formats the code.

Deployment lives here too: `infra` manages OpenTofu infrastructure, `deploy` releases a commit or rolls back, `console` hosts or connects to the self-hosted deployment service, or pairs a worker with the hosted LeptosKit app, and `verify` reproduces backend builds and checks signed deployment policies. New projects resolve their development secrets through Monosecret, and `leptoskit dev --no-secrets` uses the existing environment. The binary is a thin wrapper over the library, so tooling can drive the same commands directly.

### Features

#### Try native plugins written only in Rust

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #506](https://github.com/leptoskit/leptoskit/pull/506)

Set `native.experimental-rust-plugins = true` in `leptoskit.toml` to use the experimental pure-Rust `clipboard` and `haptics` plugins on iOS, macOS, and Android in place of their Swift and Kotlin versions. Without the setting, builds keep the Swift and Kotlin versions.

A plugin of your own can do the same in its `[package.metadata.leptoskit.runtime]` tables. With `rust-context = true`, its factory receives a `leptoskit_bridge::AppContext`, whose `platform()` lends the host's native handles: the `JavaVM` and app `Context` on Android, and the on-screen view controller on iOS and macOS. With `rust-main-thread = true`, its calls run on the platform's UI thread, which UIKit, AppKit, and most Android UI APIs require (`App::install_main` installs such a plugin by hand). Mark a platform's registration `experimental = true` to use it only when the setting is on.

### Fixes

- **Reserve private operation logs before launching the deployment command so a repeated operation cannot execute again when its logs already exist.** _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #516](https://github.com/leptoskit/leptoskit/pull/516)
- **Resolve deployment Git operations against the registered repository even when the CLI runs from a Git hook.** _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #516](https://github.com/leptoskit/leptoskit/pull/516)
- **Explain how to connect to the deployment service when its saved credential is missing on Unix.** _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #516](https://github.com/leptoskit/leptoskit/pull/516)
- **Paint italic text and per-edge borders on HarmonyOS.** The ArkUI host now applies `italic` and per-edge `border_widths` (`border-t`, `border-l-2`, …) the same way Android and GTK do, instead of leaving those strokes undrawn and italic upright. Deno render tests lock the Yoga frames from `grid_parity_probe` so absolute ArkUI placement stays aligned with the web HTML probe. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #539](https://github.com/leptoskit/leptoskit/pull/539)
- **Correct README claims about generated renderers, accessibility, runtime plugin registration, password screens, RPC macros, and interpreter updates.** Share crate summaries with Rust documentation and derive MSRV badges from Cargo through MDT templates, with a documentation drift check. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #515](https://github.com/leptoskit/leptoskit/pull/515)
- **Run recovery SQL without reading psql startup files, so local formatting settings cannot break backup verification.** _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #516](https://github.com/leptoskit/leptoskit/pull/516)
