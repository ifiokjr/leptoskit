<h1 align="center">leptoskit_plugin_brightness</h1>

<p align="center">
  <strong>leptoskit_plugin_brightness</strong> is the leptoskit plugin for scoped mobile screen brightness overrides with explicit restoration.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://docs.rs/leptoskit_plugin_brightness"><strong>Docs</strong></a> ·
  <a href="https://github.com/ifiokjr/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<p align="center">
  <a href="https://crates.io/crates/leptoskit_plugin_brightness"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_plugin_brightness.svg"></a>
  <a href="https://docs.rs/leptoskit_plugin_brightness"><img alt="docs.rs" src="https://docs.rs/leptoskit_plugin_brightness/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<br>

## Overview

Mobile screen brightness with explicit restoration. Add the crate with its `guest` feature, select `brightness` in `native.plugins`, and run `leptoskit native prepare` to register the native handler.

`get` returns `Setting::Level` or `Setting::System`. The latter means an Android window inherits the OS setting; it does not invent a level for adaptive brightness. Levels use thousandths (0..=1000) with finite fraction validation and exactly four little-endian bytes.

Android changes only the owning activity's `WindowManager.LayoutParams.screenBrightness` and needs no settings permission. The original value, including the system-default sentinel, is captured on the first set and restored by `restore` or `close`. iOS changes the current foreground scene's screen brightness, which is screen-wide and can outlive app focus. The prior brightness is restored by default on `close`; embedding apps can explicitly choose `BrightnessPlugin(restoreOnClose: false)`. Restoration is idempotent. One plugin instance must own these overrides; another component independently changing the same screen/window cannot be reconciled automatically.

Desktop screen controls and the browser are unsupported and report `Unavailable`; CSS dimming is not a screen-brightness implementation. This port does not write persistent system settings, change adaptive brightness mode, request Android `WRITE_SETTINGS`, or subscribe to brightness changes.

Wire methods: `0` get (empty input, four-byte setting), `1` set (four-byte level, empty output), `2` restore (empty input/output). The value `u32::MAX` is a read-only system-default sentinel and is rejected by set. Native close restores the captured value and rejects subsequent requests with `Cancelled`.

Reference: [Expo Brightness](https://docs.expo.dev/versions/latest/sdk/brightness/).

## How it fits

Add `leptoskit_plugin_brightness` as a direct dependency and select the `brightness` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::Brightness` rather than resolving the plugin on every call. Android changes only the activity window, iOS changes the screen and restores the previous value, and desktop and the browser return `Unavailable`.

## Use it

```rust,ignore
use leptoskit_plugin_brightness::{Level, guest::Brightness};
let brightness = Brightness::new()?;
brightness.set(Level::from_fraction(0.8)?).await?;
// When the temporary override is no longer needed:
brightness.restore().await?;
```

## Learn more

- [Crate documentation](https://docs.rs/leptoskit_plugin_brightness)
- [leptoskit documentation](https://ifiokjr.github.io/leptoskit/)
- [Repository](https://github.com/ifiokjr/leptoskit)
- [Plugin inventory](https://ifiokjr.github.io/leptoskit/platforms/plugin-inventory.html)
