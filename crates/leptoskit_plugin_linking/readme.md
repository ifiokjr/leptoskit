<h1 align="center">leptoskit_plugin_linking</h1>

<p align="center">
  <strong>leptoskit_plugin_linking</strong> is the leptoskit plugin for validated external HTTP(S), email, phone, and SMS links plus application settings, without executing shell text.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://docs.rs/leptoskit_plugin_linking"><strong>Docs</strong></a> ·
  <a href="https://github.com/ifiokjr/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<p align="center">
  <a href="https://crates.io/crates/leptoskit_plugin_linking"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_plugin_linking.svg"></a>
  <a href="https://docs.rs/leptoskit_plugin_linking"><img alt="docs.rs" src="https://docs.rs/leptoskit_plugin_linking/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<br>

## Overview

A separate Rust crate for outbound system links and application settings, with Swift and Kotlin handlers discovered by `leptoskit native prepare`.

Enable the crate's `guest` feature for an interpreted Wasm application and select `linking` in `native.plugins`. The default policy permits `http`, `https`, `mailto`, `tel` and `sms`. Custom schemes, local files and executable schemes are rejected. Destinations must be ASCII (percent-encode Unicode text), at most 8192 bytes, and contain no whitespace, control characters, escaped controls, malformed escapes, credentials or backslashes. Payloads are revalidated at native trust boundaries.

| Operation    | iOS   | Android              | macOS                        | Windows/Linux      | Web           |
| ------------ | ----- | -------------------- | ---------------------------- | ------------------ | ------------- |
| Open URL     | UIKit | `ACTION_VIEW`        | AppKit or Rust system opener | Rust system opener | `window.open` |
| Can open URL | UIKit | Package Manager      | Launch Services              | Unavailable        | Unavailable   |
| App settings | UIKit | App details settings | Unavailable                  | Unavailable        | Unavailable   |

On desktop, register `desktop::Linking` with the bridge or use `desktop::open` directly. Rust uses the `open` crate to pass the URL as a launcher argument; it never builds a shell command from the URL. System launcher operations can block briefly and should run away from the render thread. Success means the launch request was accepted, not that a destination loaded or an action such as a phone call completed.

`can_open` reports discoverable handlers. For iOS custom/system schemes, add the permitted queried schemes (for example `mailto`, `tel`, `sms`) to `LSApplicationQueriesSchemes` in the app's Info.plist. Android 11+ requires matching `<queries><intent><action android:name="android.intent.action.VIEW"/><category android:name="android.intent.category.BROWSABLE"/><data android:scheme="mailto"/></intent></queries>` entries in AndroidManifest.xml to discover handlers. A false query result can mean a missing declaration. Opening uses the system directly and does not require a successful discovery query. Query declarations are application configuration and are not generated automatically by this crate.

Browser calls must happen directly in a user gesture. `noopener,noreferrer` isolates the destination but prevents observing the opened window; a successful call cannot prove a popup escaped the browser's blocker. There is no reliable browser API for installed protocol handlers, so `web::can_open` reports `Unavailable`.

Incoming links, initial launch URLs, universal/app-link entitlements and arbitrary custom scheme policies require host lifecycle/configuration support and are not implemented here. These limitations are explicit rather than successful no-op methods.

Wire operations: method 0 opens raw UTF-8 URL bytes and returns an empty response; method 1 accepts the same payload and returns exactly one little-endian u32 (`0` or `1`); method 2 accepts and returns empty bytes for app settings. Unknown methods and unsupported operations return `Unavailable`; invalid payloads return `InvalidArgument`.

API references: [Expo Linking](https://docs.expo.dev/versions/latest/sdk/linking/), [Tauri Opener](https://v2.tauri.app/plugin/opener/), and [Electron shell](https://www.electronjs.org/docs/latest/api/shell). This is an idiomatic Rust API over native services, not a source or API compatibility claim.

## How it fits

Add `leptoskit_plugin_linking` as a direct dependency and select the `linking` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::Linking` rather than resolving the plugin on every call. Incoming deep links need host lifecycle integration and are not handled by this crate.

## Use it

```rust,ignore
use leptoskit_plugin_linking::{Link, guest::Linking};

let linking = Linking::new()?;
let destination = Link::parse("https://example.com/help")?;
linking.open(&destination).await?;
```

## Learn more

- [Crate documentation](https://docs.rs/leptoskit_plugin_linking)
- [leptoskit documentation](https://ifiokjr.github.io/leptoskit/)
- [Repository](https://github.com/ifiokjr/leptoskit)
- [Plugin inventory](https://ifiokjr.github.io/leptoskit/platforms/plugin-inventory.html)
