<h1 align="center">leptoskit_plugin_os</h1>

<p align="center">
  <strong>leptoskit_plugin_os</strong> is the leptoskit plugin for the host operating system and CPU architecture as seen from inside a Wasm guest.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://docs.rs/leptoskit_plugin_os"><strong>Docs</strong></a> ·
  <a href="https://github.com/ifiokjr/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<p align="center">
  <a href="https://crates.io/crates/leptoskit_plugin_os"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_plugin_os.svg"></a>
  <a href="https://docs.rs/leptoskit_plugin_os"><img alt="docs.rs" src="https://docs.rs/leptoskit_plugin_os/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<br>

## Overview

The operating system and CPU architecture the app is actually running on. A native leptoskit app runs its interface as a Wasm guest, and from inside the guest everything looks like `wasm32`; this plugin asks the native host instead.

The values come from compile-time constants of the host build, so a call needs no allocation and no platform API:

| Field          | Values                                                                                   |
| -------------- | ---------------------------------------------------------------------------------------- |
| `platform`     | `ios`, `android`, `macos`, `windows`, `linux`, or `web`                                  |
| `architecture` | Rust's `std::env::consts::ARCH`, such as `aarch64`, `x86_64`, or `wasm32` in the browser |

Rust code can read them directly with `leptoskit_plugin_os::info()`. In a browser build the plugin reports `web` and `wasm32` honestly; it never parses a user-agent string, which is spoofable and says nothing reliable about the device. For model, OS version, and memory, use [`leptoskit_plugin_device`](https://github.com/ifiokjr/leptoskit/tree/main/crates/leptoskit_plugin_device).

Add the crate as a direct app dependency and select `"os"` in `native.plugins`. The plugin is implemented in Rust (`Os`, declared with `rust-type` in the crate's Cargo metadata), so there are no Swift or Kotlin handlers and the host registers it on every native target. A guest has no typed wrapper; it calls the one method through the bridge client and decodes the reply with the bridge codec.

Enable `leptoskit_bridge`'s `guest` feature in the guest crate. Method `0` (`INFO`) takes an empty payload and returns two length-prefixed UTF-8 strings, platform then architecture. A payload on `INFO` returns `InvalidArgument`; any other method returns `Unavailable`. The crate's tests cover the reply encoding and both rejections, and run with `cargo test -p leptoskit_plugin_os`.

## How it fits

Add `leptoskit_plugin_os` as a direct dependency and select the `os` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. The values come from compile-time constants of the host build, so the guest sees the real platform even though its own target is wasm32.

## Use it

```rust,ignore
use leptoskit_bridge::codec::Decoder;
use leptoskit_bridge::guest::Client;
use leptoskit_plugin_os::INFO;

let os = Client::new("os")?;
let reply = os.call(INFO, &[]).await?;
let mut decoder = Decoder::new(&reply);
let platform = decoder.string()?;
let architecture = decoder.string()?;
decoder.finish()?;
```

## Learn more

- [Crate documentation](https://docs.rs/leptoskit_plugin_os)
- [leptoskit documentation](https://ifiokjr.github.io/leptoskit/)
- [Repository](https://github.com/ifiokjr/leptoskit)
- [Plugin inventory](https://ifiokjr.github.io/leptoskit/platforms/plugin-inventory.html)
