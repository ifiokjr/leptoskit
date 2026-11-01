<h1 align="center">leptoskit_plugin_screen_capture</h1>

<p align="center">
  <strong>leptoskit_plugin_screen_capture</strong> is the leptoskit plugin for window-scoped capture prevention and capture observation as separate capabilities.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://docs.rs/leptoskit_plugin_screen_capture"><strong>Docs</strong></a> ·
  <a href="https://github.com/ifiokjr/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<p align="center">
  <a href="https://crates.io/crates/leptoskit_plugin_screen_capture"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_plugin_screen_capture.svg"></a>
  <a href="https://docs.rs/leptoskit_plugin_screen_capture"><img alt="docs.rs" src="https://docs.rs/leptoskit_plugin_screen_capture/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<br>

## Overview

Capture prevention and capture observation have separate capabilities. This plugin never captures pixels, reads a screenshot file, or scans the photo library. Each plugin instance belongs to its host's Activity/window or explicit Apple `NativePluginContext`; it does not search global windows.

| Platform                   | Prevention                       | Recording/mirroring state | Screenshot notification       |
| -------------------------- | -------------------------------- | ------------------------- | ----------------------------- |
| Android                    | Owned window `FLAG_SECURE` lease | Unavailable / unknown     | Android 14+ Activity callback |
| iOS 17+                    | Unavailable                      | Owned scene capture trait | App-wide UIKit notification   |
| macOS, Windows, Linux, web | Unavailable                      | Unavailable               | Unavailable                   |

The iOS screenshot notification cannot identify which app window was captured. It reports app-wide scope, and only a pending observer with a foreground owned scene receives it. Screenshot notifications happen after capture. There is no private text-field/layer workaround pretending to prevent iOS screenshots. Browser screen-sharing permission APIs provide a different contract and are not used as a substitute for prevention or screenshot detection.

Add the crate as a direct dependency and `"screen-capture"` to `native.plugins`. Android registration declares the normal install-time permission `android.permission.DETECT_SCREEN_CAPTURE`. Screenshot observation on Android 14+ returns `PermissionDenied` if the host omits that permission. Earlier Android versions return `Unavailable`; no photo/storage permissions are requested. The Android OS can show a screenshot-detection notice to the user. The host app should explain its use before starting observation.

A prevention request remains pending for its entire lifetime. Pending means no failure has arrived, not an acknowledgement that the secure bit has been set. The host's snapshot can confirm the current window flag. Dropping the future, closing the plugin or destroying its Activity releases the lease. At most 64 leases may share a window, including across plugin instances. The first lease saves the original secure bit; the final release restores that bit exactly while preserving changes to unrelated window flags. Other code changing the same secure bit must coordinate with these leases. A lease may start before the window is shown or while a dialog has focus, so content is protected from its first frame. Screenshot observation needs the Activity's content to be visible, not focused. Android prevention remains active when the Activity stops, protecting recents snapshots until release/destruction. This is an OS window policy, not a guarantee against a compromised OS or an external camera.

Only one observation request may be pending per plugin (`Busy` otherwise). Native subscriptions exist only while that request is pending and are removed on completion, cancellation, registration failure, native background transition or close. Capture-state observation resamples after installing the scene trait observer; intermediate transitions may coalesce. Screenshot events between requests are deliberately not buffered or replayed. The Apple scene must remain the same explicitly owned scene; Android observes only its own Activity.

The binary protocol uses little-endian u32 fields with strict lengths:

| Method         | Request                      | Reply                                                      |
| -------------- | ---------------------------- | ---------------------------------------------------------- |
| 0 capabilities | empty                        | version=1, prevention 0/1, capture scope, screenshot scope |
| 1 snapshot     | empty                        | version=1, capture state, secure flag                      |
| 2 wait         | kind, previous capture state | version=1, kind, capture state                             |
| 3 prevent      | empty                        | stays pending; cancellation/error only                     |

Scope is unavailable/window/scene/application = 0/1/2/3. Capture state is unknown/inactive/active = 0/1/2. Secure flag is unknown/off/on = 0/1/2. Wait kind 0 means capture changed; kind 1 means screenshot and requires previous state 0. Screenshot replies always carry state 0. Rust rejects replies for the wrong requested kind, unchanged capture-change replies, unsupported versions, unknown values, truncation and trailing data.

The implementation follows the capability areas exposed by [Expo ScreenCapture](https://docs.expo.dev/versions/latest/sdk/screen-capture/), using Android's public [secure-window and screenshot callback APIs](https://developer.android.com/about/versions/14/features/screenshot-detection) and Apple's public [scene capture trait](https://developer.apple.com/documentation/uikit/uitraitcollection/scenecapturestate). Native tests use synthetic notifications/subscriptions or an owned fixture Activity. They never capture user apps or data. Actual screenshot/recording notifications and OS compositor enforcement still require device verification.

## How it fits

Add `leptoskit_plugin_screen_capture` as a direct dependency and select the `screen-capture` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::ScreenCapture` rather than resolving the plugin on every call. The plugin never captures pixels or reads screenshots; Android can protect its owned window, while iOS can observe scene capture and app screenshots without prevention.

## Use it

```rust,ignore
use leptoskit_plugin_screen_capture::{guest::ScreenCapture, Watch};
let capture = ScreenCapture::new()?;
let capabilities = capture.capabilities().await?;
let event = capture.wait(Watch::Screenshot).await?;
```

## Learn more

- [Crate documentation](https://docs.rs/leptoskit_plugin_screen_capture)
- [leptoskit documentation](https://ifiokjr.github.io/leptoskit/)
- [Repository](https://github.com/ifiokjr/leptoskit)
- [Plugin inventory](https://ifiokjr.github.io/leptoskit/platforms/plugin-inventory.html)
