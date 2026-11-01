<h1 align="center">leptoskit_plugin_permissions</h1>

<p align="center">
  <strong>leptoskit_plugin_permissions</strong> is the leptoskit plugin for explicit camera and microphone permission queries and requests.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://docs.rs/leptoskit_plugin_permissions"><strong>Docs</strong></a> ·
  <a href="https://github.com/ifiokjr/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<p align="center">
  <a href="https://crates.io/crates/leptoskit_plugin_permissions"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_plugin_permissions.svg"></a>
  <a href="https://docs.rs/leptoskit_plugin_permissions"><img alt="docs.rs" src="https://docs.rs/leptoskit_plugin_permissions/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<br>

## Overview

Explicit camera and microphone permission queries and requests for native apps and browsers. This crate does not claim to cover permissions with different OS flows, such as location, photos, notifications, Bluetooth, motion or background access.

Add the crate as a direct dependency and select `permissions` in `native.plugins`. Runtime registration grants access to this plugin's methods; the OS still makes its own independent permission decision.

## Native setup and lifecycle

On iOS and macOS, add meaningful `NSCameraUsageDescription` and `NSMicrophoneUsageDescription` values to your app-owned `hosts/ios/Info.plist` or macOS Info.plist for capabilities you request. Missing or blank purpose strings return `InvalidArgument` before Apple's permission API could terminate the app. Sandboxed macOS apps also need the applicable camera/audio-input entitlements. The plugin does not invent purpose text or grant entitlements. Queries use AVFoundation without prompting; requests use the platform authorization dialog. Denied/restricted permissions are reported directly, and Settings is never opened implicitly. Only one new Apple permission dialog can be outstanding per handler; concurrent requests return `Busy`. Closing the handler rejects new work. OS dialogs already displayed cannot be dismissed by canceling a Rust future.

Android metadata declares CAMERA and RECORD_AUDIO. Generated `MainActivity` implements `NativePermissionHost` and forwards permission results to its `NativePermissionBroker`. Custom activities must provide those same hooks:

- Implement `requestNativePermissions` by calling the activity-owned broker.
- Forward `onRequestPermissionsResult` to `broker.onResult`.
- Call `restoreState` before starting the guest and `saveState` from `onSaveInstanceState`.
- Match `setActive(true/false)` to `onResume/onPause`; inactive or already-saved activities reject new prompts with `Busy`.
- Call `broker.close()` during activity destruction.

The broker reserves request codes `0x4C00` through `0xFFFF`, allows one OS request at a time, and retains a canceled request's slot until its OS result arrives. This prevents a late result from completing a different request. Codes increase monotonically; the saved allocator watermark prevents reuse across Activity/process restoration. After the range is consumed, requests fail with `Exhausted` instead of reusing an identifier. Closing it cancels pending callers. Missing manifest declarations fail before a request is issued.

Android's public status APIs cannot always distinguish never-requested from permanently denied access. The plugin reports `Unknown` and unknown `can_prompt` in that case. A request that returns denial reports `Denied`; its `can_prompt` value uses the platform rationale result. Do not interpret unknown as permission granted or add a prompt loop. On Apple and browsers, states include `NotDetermined`, `Denied`, `Granted`, and `Restricted` where the platform reports them. A grant does not prove the device has the requested hardware or that it is currently available.

## Browser behavior

`web::status` uses the Permissions API. Unsupported camera/microphone queries return `Unavailable`; the API does not infer status from a guessed browser name. `web::request` uses `getUserMedia`, then immediately stops every media track. Its cleanup task survives cancellation of the caller future, so a late grant cannot leave a camera or microphone capturing. Browser policy and secure-context requirements apply; denials return `PermissionDenied`, missing hardware or APIs return `Unavailable`, and busy hardware can return `Busy`. Actual capture and recording belong in separate plugins.

## Wire format and verification

Methods: status `0`, request `1`. Input is one little-endian u32: camera `0` or microphone `1`. Output is two u32 values: state (unknown `0`, not determined `1`, denied `2`, granted `3`, restricted `4`) and can-prompt (unknown `0`, no `1`, yes `2`). Invalid lengths, unsupported kinds, enum values and trailing bytes fail. No JSON encoding or repeated plugin-name lookup occurs on the guest call path.

Rust tests validate the protocol and unknown-state handling. Crate-owned Android instrumentation tests exercise the real permission dialog and callback routing, plus closed/invalid broker requests. The dialog test uses an already-granted permission if the test application has been run before; use a freshly installed test application to exercise the OS prompt.

References: [Apple capture authorization](https://developer.apple.com/documentation/avfoundation/capture-setup/requesting-authorization-to-capture-and-save-media), [Android runtime permissions](https://developer.android.com/training/permissions/requesting), [Media capture](https://developer.mozilla.org/en-US/docs/Web/API/MediaDevices/getUserMedia), [Expo permission conventions](https://docs.expo.dev/guides/permissions/).

## How it fits

Add `leptoskit_plugin_permissions` as a direct dependency and select the `permissions` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::Permissions` rather than resolving the plugin on every call. Querying status never shows a prompt; request only after the user chooses, and use the dedicated location, notification, and photo APIs for permissions with different OS flows.

## Use it

```rust,ignore
use leptoskit_plugin_permissions::{Permission, Status, guest::Permissions};

let permissions = Permissions::new()?;
let current = permissions.status(Permission::Camera).await?;
// Call request only after the user chooses a feature needing the camera.
let result = permissions.request(Permission::Camera).await?;
if result.status == Status::Granted {
    // Start the separate camera API; permission does not guarantee hardware.
}
```

## Learn more

- [Crate documentation](https://docs.rs/leptoskit_plugin_permissions)
- [leptoskit documentation](https://ifiokjr.github.io/leptoskit/)
- [Repository](https://github.com/ifiokjr/leptoskit)
- [Plugin inventory](https://ifiokjr.github.io/leptoskit/platforms/plugin-inventory.html)
