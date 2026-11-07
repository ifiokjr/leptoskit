<h1 align="center">leptoskit_plugin_device_policy</h1>

<p align="center">
  <strong>leptoskit_plugin_device_policy</strong> is the leptoskit plugin for explicit Android Device Admin / Device Policy Manager capabilities such as lock, password rules, camera disable, and confirmed wipe.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://docs.rs/leptoskit_plugin_device_policy"><strong>Docs</strong></a> ·
  <a href="https://github.com/leptoskit/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<!-- {=crateBadges|replace:"CRATE_NAME":"leptoskit_plugin_device_policy"} -->

<p align="center">
  <a href="https://crates.io/crates/leptoskit_plugin_device_policy"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_plugin_device_policy.svg"></a>
  <a href="https://docs.rs/leptoskit_plugin_device_policy"><img alt="docs.rs" src="https://docs.rs/leptoskit_plugin_device_policy/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<!-- {/crateBadges} -->

<br>

## Overview

Explicit Android Device Admin / Device Policy Manager (DPM) capabilities. Query admin state without prompting, ask the user to enable device admin after they choose a feature, then call only the privileged action you need: lock the screen, set password quality or minimum length, disable the camera, or wipe the device with an explicit confirmation token.

Add the crate as a direct dependency and `"device-policy"` to `native.plugins`. Android registration needs an Activity context and the host's activity-result broker. Apple, Windows, Linux, the terminal, HarmonyOS, and browsers return `Unavailable`; they never invent an admin grant.

Device admin is dangerous and tightly constrained by Google Play. Consumer apps usually cannot ship Device Admin for general MDM-style control. Prefer the least policy your product needs, declare only those policies in an app-owned `res/xml/device_admin.xml`, and expect Play review to reject unrelated wipe or password APIs. Device Owner / Profile Owner / Android Management API enrollment is outside this plugin.

The app owns the manifest receiver and the policy XML. The plugin ships `DevicePolicyReceiver` but does not inject it into your manifest:

```xml
<receiver
  android:name="dev.leptoskit.plugins.devicepolicy.DevicePolicyReceiver"
  android:permission="android.permission.BIND_DEVICE_ADMIN"
  android:exported="true">
  <meta-data
    android:name="android.app.device_admin"
    android:resource="@xml/device_admin" />
  <intent-filter>
    <action android:name="android.app.action.DEVICE_ADMIN_ENABLED" />
  </intent-filter>
</receiver>
```

Start with the smallest policy file that matches the methods you call. A lock-only app should not declare wipe:

```xml
<?xml version="1.0" encoding="utf-8"?>
<device-admin xmlns:android="http://schemas.android.com/apk/res/android">
  <uses-policies>
    <force-lock />
  </uses-policies>
</device-admin>
```

Status never prompts. `request_activation` requires a non-blank explanation and opens Android's system consent UI; it does not auto-enable admin. Privileged methods return `PermissionDenied` until the user enables the receiver. Wipe requires `WipeRequest::factory_reset()` so accidental empty payloads cannot factory-reset a device. Password quality is never upgraded implicitly: you set the exact quality you mean.

iOS, macOS, and other non-Android hosts register an unavailable adapter so the same guest dependency compiles everywhere. Native tests never activate device admin, lock a device, or wipe data.

## How it fits

<!-- {=p_device_policySummary} -->

Add `leptoskit_plugin_device_policy` as a direct dependency and select the `device-policy` plugin in `native.plugins`; supported generated hosts discover it from Cargo metadata. Generated Windows, Linux, and terminal hosts currently omit runtime plugins; use explicit registration in a custom host there. Guests enable the `guest` feature and reuse `guest::DevicePolicy` rather than resolving the plugin on every call. Only Android implements Device Admin / Device Policy Manager capabilities; every other platform returns `Unavailable`. Status never prompts. Privileged actions require an active device admin the user enabled, and wipe requires an explicit confirmation token.

<!-- {/p_device_policySummary} -->

## Use it

```rust,ignore
use leptoskit_plugin_device_policy::{
  guest::DevicePolicy, ActivationRequest, PasswordQuality, WipeRequest,
};

let policy = DevicePolicy::new()?;
let status = policy.status().await?;
if status.state != leptoskit_plugin_device_policy::AdminState::Active {
  let request = ActivationRequest::new(
    "Allow this app to lock the screen if this phone is lost.",
  )?;
  policy.request_activation(&request).await?;
}
policy.lock_now().await?;
policy.set_password_quality(PasswordQuality::Numeric).await?;
// Irreversible. Call only after a dedicated in-app confirmation.
policy.wipe_data(WipeRequest::factory_reset()).await?;
```

## Learn more

- [Crate documentation](https://docs.rs/leptoskit_plugin_device_policy)
- [leptoskit documentation](https://leptoskit.dev/docs/)
- [Repository](https://github.com/leptoskit/leptoskit)
- [Plugin inventory](https://leptoskit.dev/docs/platforms/plugin-inventory.html)
