<h1 align="center">leptoskit_plugin_arguments</h1>

<p align="center">
  <strong>leptoskit_plugin_arguments</strong> is the leptoskit plugin for immutable launch-argument snapshots with typed, non-exiting clap parsing.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://docs.rs/leptoskit_plugin_arguments"><strong>Docs</strong></a> ·
  <a href="https://github.com/ifiokjr/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<p align="center">
  <a href="https://crates.io/crates/leptoskit_plugin_arguments"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_plugin_arguments.svg"></a>
  <a href="https://docs.rs/leptoskit_plugin_arguments"><img alt="docs.rs" src="https://docs.rs/leptoskit_plugin_arguments/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<br>

## Overview

Launch arguments for desktop hosts and their guests, with an immutable snapshot and typed parsing through `clap::Parser::try_parse_from`. Parsing never exits the application: help, version, and validation errors remain structured clap errors for the app to display.

`Snapshot::capture()` reads process arguments once on macOS, Linux, and Windows. `Snapshot::new(program, arguments)` builds an explicit snapshot, including for application activation events. Native `OsString` values preserve non-UTF-8 arguments. `Snapshot::parse::<YourOptions>()` parses the application's own clap model, including normal option terminator and quoting semantics. Arguments are already split by the OS; this crate does not split or evaluate shell command strings. `program()` is argv[0], not a trusted executable identity.

To disclose a snapshot to a guest, construct `Arguments::new(&snapshot)` and register/grant it on the native bridge. Its `SNAPSHOT` method returns a pre-encoded binary snapshot by cloning `Bytes`. The `guest` feature exposes `guest::ArgumentsClient`. The host chooses the disclosure set explicitly; there is no default constructor that silently forwards potentially sensitive command-line values.

The wire contract is program UTF-8 bytes, argument count, then argument UTF-8 bytes, using the shared length-prefixed codec. Snapshots are limited to 4,096 arguments and 1 MiB of encoded data. Non-UTF-8 arguments remain usable natively but return `NonUtf8` when encoding; they are never replaced lossily. Invalid UTF-8, truncation, trailing bytes, and excessive counts are rejected during decoding.

Mobile and browser process capture returns `Unavailable`. Explicit snapshots and parsing remain usable on those targets, including snapshots received from a desktop host. This does not implement OS activation delivery, deep links, or Electron command-line switches; those belong to application lifecycle and linking.

References: [Tauri CLI](https://v2.tauri.app/plugin/cli/), [Electron process arguments](https://www.electronjs.org/docs/latest/api/process), [clap typed parsing](https://docs.rs/clap/latest/clap/trait.Parser.html#method.try_parse_from).

## How it fits

Add `leptoskit_plugin_arguments` as a direct dependency and expose the plugin explicitly from the native host; it has no `native.plugins` entry. Guests enable the `guest` feature and reuse `guest::ArgumentsClient` rather than resolving the plugin on every call. The host chooses which arguments to disclose; mobile and browser process capture returns `Unavailable`, while explicit snapshots and parsing still work.

## Use it

```rust,ignore
use leptoskit_plugin_arguments::guest::ArgumentsClient;

let arguments = ArgumentsClient::new()?;
let snapshot = arguments.snapshot().await?;
```

## Learn more

- [Crate documentation](https://docs.rs/leptoskit_plugin_arguments)
- [leptoskit documentation](https://ifiokjr.github.io/leptoskit/)
- [Repository](https://github.com/ifiokjr/leptoskit)
- [Plugin inventory](https://ifiokjr.github.io/leptoskit/platforms/plugin-inventory.html)
