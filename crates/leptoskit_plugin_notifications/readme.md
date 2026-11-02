<h1 align="center">leptoskit_plugin_notifications</h1>

<p align="center">
  <strong>leptoskit_plugin_notifications</strong> is the leptoskit plugin for immediate local system notifications with explicit permission requests, bounded messages, and cancellation.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://docs.rs/leptoskit_plugin_notifications"><strong>Docs</strong></a> ·
  <a href="https://github.com/ifiokjr/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<p align="center">
  <a href="https://crates.io/crates/leptoskit_plugin_notifications"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_plugin_notifications.svg"></a>
  <a href="https://docs.rs/leptoskit_plugin_notifications"><img alt="docs.rs" src="https://docs.rs/leptoskit_plugin_notifications/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<br>

## Overview

Immediate local notifications with explicit permission requests, bounded binary messages and cancellation. A successful call means the platform accepted the request. It does not prove that a banner appeared or the user saw it: Focus modes, channel settings, foreground presentation policy and desktop services control delivery.

| Platform    | Implementation                          | Scope                                                                                   |
| ----------- | --------------------------------------- | --------------------------------------------------------------------------------------- |
| iOS / macOS | Swift `UNUserNotificationCenter`        | Permission query/request, immediate posting, named cancellation, namespace cancellation |
| Android 26+ | Kotlin `NotificationManager`            | The above, plus channels and Android 13 notification permission                         |
| Browser     | Rust `web` module using `Notification`  | Explicit permission request, posting and cancellation through retained handles          |
| Linux       | Rust `linux` module using session D-Bus | Immediate posting and cancellation through retained handles                             |
| Windows     | Unavailable                             | No Windows toast backend yet                                                            |

Scheduled notifications, remote push registration, service-worker notifications, badges, attachments, click/action routing and delivery receipts are not implemented. Unsupported operations return `Unavailable` where the backend exposes them. Linux and browser APIs are called directly from Rust; they are not native bridge registrations.

## Native registration and use

The crate metadata provides Apple and Android registration information for the LeptosKit generator. Android requests `android.permission.POST_NOTIFICATIONS` in the generated manifest and uses the generated activity's `NativePermissionHost` broker. A custom Android host must implement that interface to request runtime permission. Querying permission and posting do not need the permission broker.

Enable the `guest` feature in Wasm guest code and grant the methods the app needs in the host policy. The guest client caches its numeric plugin ID.

Channel configuration is Android-only; do not call it unconditionally on Apple. Apple validates the portable channel identifier but does not use Android channels. Android channel importance and sound become user-controlled after channel creation; configuring the same ID does not override their choices. Notifications are silent on Apple, browser and Linux; Android sound is an explicit channel option. The Android constructor accepts a small-icon resource; its default is the system information icon. Apps should supply a suitable monochrome notification icon when registering a custom instance.

Apple requires a properly packaged application with a bundle identity. This plugin leaves the application's notification center delegate in place. Foreground banner presentation requires that delegate's normal `willPresent` policy; this crate does not claim visible foreground delivery or take over click handling. Android does not currently install a content `PendingIntent`, so tapping a notification does not route to an application screen.

Android permission `Unknown` means the OS cannot distinguish an initial unrequested permission from a denied permission with no rationale. It is not a claim that another prompt is possible. Apple distinguishes not-determined, provisional and ephemeral authorization.

## Lifetime and cancellation

Native IDs use the `leptoskit.notifications.` namespace. A completed post with the same ID replaces the previous notification where the OS supports replacement. Use one registered plugin instance per application and distinct IDs for independent callers; cancellation is application-wide within that namespace. Cancel-all leaves notifications from other providers untouched.

Apple limits in-flight posts to 64. A concurrent post with the same ID returns `Busy` until the prior OS call completes. Cancelling a post, cancelling its bridge future or closing the plugin retracts pending work, including a late result from an OS call that cannot be aborted. Once posting has completed, closing the plugin does not dismiss delivered notifications. Cancelling a permission request cannot close the OS permission dialog or undo an authorization choice.

Browser permission requests need a secure context and usually a user gesture. Browsers that require service-worker delivery report `Unavailable` from `show`; this module does not install a service worker. Retain the returned `web::Handle` to cancel it. Dropping a handle preserves delivery. Browser tags provide replacement semantics subject to browser support.

Linux `Notifications::new` connects to the session bus without posting anything. The service assigns a numeric ID returned inside the cancellation handle; the portable ID is not used to replace notifications. Dropping a handle preserves delivery. Calls can block up to five seconds and belong off the rendering thread. The body is escaped for the desktop notification protocol's markup. App display names are metadata, not trusted application identity. Linux desktop permission prompting is not standardized, so this module does not manufacture a permission state.

## Binary protocol

All integers are little-endian `u32`. Strings are `u32` byte lengths followed by strict UTF-8. Every decoder rejects trailing bytes, truncated fields, invalid UTF-8 and values outside their domain before calling a platform API.

| ID | Method             | Request                                                   | Response         |
| -- | ------------------ | --------------------------------------------------------- | ---------------- |
| 0  | Permission         | Empty                                                     | Permission `u32` |
| 1  | Request permission | Empty                                                     | Permission `u32` |
| 2  | Show               | ID, title, body, channel strings                          | Empty            |
| 3  | Cancel             | ID string                                                 | Empty            |
| 4  | Configure channel  | ID, name, description strings, importance and sound `u32` | Empty            |
| 5  | Delete channel     | ID string                                                 | Empty            |
| 6  | Cancel all         | Empty                                                     | Empty            |

IDs contain 1–64 ASCII letters, digits, `.`, `_` or `-`. Titles have at most 256 UTF-8 bytes, bodies 4096, channel names 128 and descriptions 1024. Titles and names must contain non-whitespace text. Control characters are rejected except line feeds and tabs in bodies/descriptions. Sound is exactly 0 or 1; importance is Low=1, Default=2 or High=3. Permission is Unknown=0, NotDetermined=1, Denied=2, Granted=3, Provisional=4 or Ephemeral=5.

## Verification and references

Rust tests cover binary validation and truncation. Apple tests use an injected service to verify late completion, cancellation and close without posting system notifications. The browser test replaces `Notification` with a controlled fake and exercises explicit permission, payload fields, cancellation and unsupported service-worker-only behavior. The Android instrumentation suite posts silent notifications only in a disposable emulator fixture and checks replacement, namespace cancellation and close against `NotificationManager.activeNotifications`.

API scope is informed by [Expo Notifications](https://docs.expo.dev/versions/latest/sdk/notifications/), [Tauri Notification](https://v2.tauri.app/plugin/notification/) and [Electron Notification](https://www.electronjs.org/docs/latest/api/notification). Platform contracts come from [Apple authorization](https://developer.apple.com/documentation/usernotifications/asking-permission-to-use-notifications), [Android notification permission](https://developer.android.com/develop/ui/views/notifications/notification-permission) and the [Freedesktop notification protocol](https://specifications.freedesktop.org/notification/latest/protocol.html).

## How it fits

Add `leptoskit_plugin_notifications` as a direct dependency and select the `notifications` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::Notifications` rather than resolving the plugin on every call. A successful call means the OS accepted the request, not that a banner appeared; focus modes and user settings can still suppress it.

## Use it

```rust,ignore
use leptoskit_plugin_notifications::{Channel, Identifier, Importance, Notification};
use leptoskit_plugin_notifications::guest::Notifications;

let client = Notifications::new()?;
let channel_id = Identifier::new("downloads")?;
// Android only: create the channel before requesting permission or posting.
let channel = Channel::new(channel_id.clone(), "Downloads", "", Importance::Low, false)?;
client.configure_channel(&channel).await?;
let permission = client.request_permission().await?;
// Inspect permission before posting; requesting can be denied.
let notification = Notification::new(
    Identifier::new("download-1")?,
    "Download complete",
    "Your file is ready.",
    channel_id,
)?;
client.show(&notification).await?;
client.cancel(notification.id()).await?;
```

## Learn more

- [Crate documentation](https://docs.rs/leptoskit_plugin_notifications)
- [leptoskit documentation](https://ifiokjr.github.io/leptoskit/)
- [Repository](https://github.com/ifiokjr/leptoskit)
- [Plugin inventory](https://ifiokjr.github.io/leptoskit/platforms/plugin-inventory.html)
