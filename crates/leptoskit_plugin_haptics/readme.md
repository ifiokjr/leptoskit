<h1 align="center">leptoskit_plugin_haptics</h1>

<p align="center">
  <strong>leptoskit_plugin_haptics</strong> is the leptoskit plugin for selection, impact, and notification feedback on native actuators.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://docs.rs/leptoskit_plugin_haptics"><strong>Docs</strong></a> ·
  <a href="https://github.com/ifiokjr/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<p align="center">
  <a href="https://crates.io/crates/leptoskit_plugin_haptics"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_plugin_haptics.svg"></a>
  <a href="https://docs.rs/leptoskit_plugin_haptics"><img alt="docs.rs" src="https://docs.rs/leptoskit_plugin_haptics/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<br>

## Overview

Selection, impact and notification feedback with binary plugin messages.

- iOS uses UIKit feedback generators.
- Android uses `View.performHapticFeedback`, respecting the user's settings without requesting vibration permission. The Android-specific method accepts public `HapticFeedbackConstants` supported by the current API level.
- macOS uses its trackpad haptic performer. Its three patterns approximate the richer mobile styles.
- Browsers use the Vibration API when available and allowed by user activation/device policy.
- Windows and Linux return `Unavailable`; this crate does not assume a game controller or other actuator exists.

Add the crate as a direct app dependency and select `"haptics"` in `native.plugins`. Native registration is automatic. Interpreter guests enable the `guest` feature and construct `guest::Haptics` once to cache its numeric plugin ID.

Success means the request was accepted. The OS may suppress physical feedback because of settings, power state, hardware or foreground restrictions.

The portable binary contract uses method 0 with no payload for selection, method 1 with one little-endian `u32` impact style (light, medium, heavy, rigid, soft), and method 2 with one `u32` notification kind (success, warning, error). Method 3 accepts an Android constant. Successful replies are empty. Truncated, trailing or unknown portable values fail before requesting hardware feedback.

References: [Expo Haptics](https://docs.expo.dev/versions/latest/sdk/haptics/), [Android feedback constants](https://developer.android.com/reference/android/view/HapticFeedbackConstants), [macOS haptic performer](https://developer.apple.com/documentation/appkit/nshapticfeedbackmanager).

## How it fits

Add `leptoskit_plugin_haptics` as a direct dependency and select the `haptics` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::Haptics` rather than resolving the plugin on every call. iOS uses UIKit feedback generators, Android uses platform haptic feedback, and macOS approximates the mobile styles with its trackpad performer; browsers use the Vibration API where allowed.

## Use it

```toml
# In a native host, select `haptics` in `native.plugins`.
[dependencies]
leptoskit_plugin_haptics = { version = "0.1", features = ["guest"] }
```

## Learn more

- [Crate documentation](https://docs.rs/leptoskit_plugin_haptics)
- [leptoskit documentation](https://ifiokjr.github.io/leptoskit/)
- [Repository](https://github.com/ifiokjr/leptoskit)
- [Plugin inventory](https://ifiokjr.github.io/leptoskit/platforms/plugin-inventory.html)
