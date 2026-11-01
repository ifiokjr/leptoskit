<h1 align="center">leptoskit_plugin_battery</h1>

<p align="center">
  <strong>leptoskit_plugin_battery</strong> is the leptoskit plugin for battery charge, charging state, and system power-saving snapshots with cancellable change observation.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://docs.rs/leptoskit_plugin_battery"><strong>Docs</strong></a> ·
  <a href="https://github.com/ifiokjr/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<p align="center">
  <a href="https://crates.io/crates/leptoskit_plugin_battery"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_plugin_battery.svg"></a>
  <a href="https://docs.rs/leptoskit_plugin_battery"><img alt="docs.rs" src="https://docs.rs/leptoskit_plugin_battery/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<br>

## Overview

Battery charge, charging state, and system power-saving state, with Swift and Kotlin handlers for generated mobile apps. Unknown readings remain `None`; an iOS simulator is not reported as a fully charged physical battery.

Add the crate to the native host and select `"battery"` in `native.plugins`. The generator discovers `BatteryPlugin` from Cargo package metadata. No mobile permission prompt is needed.

Guest applications enable the `guest` feature.

Dropping `wait_for_change` cancels the native request and removes its observers. Only one waiter may be active per native plugin; another returns `Busy`. Each wait installs observers before sampling, so changes between calls are detected. The result is the latest state: intermediate transitions can be coalesced, and a slow consumer does not accumulate an event backlog. Closing the host cancels outstanding subscriptions before releasing the plugin.

| Platform              | Snapshots                                             | Change observation                                             | Power saving   |
| --------------------- | ----------------------------------------------------- | -------------------------------------------------------------- | -------------- |
| iOS                   | `UIDevice`                                            | Battery and process power-state notifications                  | Low Power Mode |
| Android               | Battery broadcast                                     | Battery and power-save broadcasts                              | Power Saver    |
| macOS Swift host      | IOKit internal battery                                | IOKit run-loop and process power-state notifications           | Low Power Mode |
| macOS, Windows, Linux | `desktop::batteries()` through OS battery enumeration | `desktop::Watcher`, explicit polling at least one second apart | Unknown        |
| Browser               | `web::snapshot()` when Battery Status API exists      | `web::wait_for_change()`, listeners removed on future drop     | Unknown        |

The desktop `Battery` bridge adapter exposes snapshots only. `Watcher::poll` performs synchronous OS I/O and should run off the UI thread; it creates no background thread and does not claim to use OS notifications. Multiple desktop batteries are returned individually. The bridge snapshot uses the first battery. Generated macOS apps use the Swift IOKit implementation, including native notifications. Other desktop Rust hosts can use `desktop::batteries()` and `desktop::Watcher`.

Android additionally provides `guest::Battery::optimization_enabled()` for the application's battery-optimization exemption policy. This is separate from the device-wide Power Saver state. Other platforms return `Unavailable`. Browsers without the API return `Unavailable`; browser security-policy rejection returns `PermissionDenied`, and other native failures remain `Native`.

Snapshot payloads are exactly 16 bytes: four little-endian `u32` fields for presence (`0` unknown, `1` absent, `2` present), charge in thousandths (`u32::MAX` unknown), state (`0` unknown, `1` discharging, `2` charging, `3` full), and power-saving (`0` unknown, `1` off, `2` on). All decoders reject invalid values and trailing bytes. Method `0` reads a snapshot; method `1` waits for a snapshot different from the supplied 16-byte previous value; Android method `2` returns the optimization policy as a four-byte `0` or `1`.

API reference: [Expo Battery](https://docs.expo.dev/versions/latest/sdk/battery/), [Apple battery monitoring](https://developer.apple.com/documentation/uikit/uidevice/isbatterymonitoringenabled), and [Android battery monitoring](https://developer.android.com/training/monitoring-device-state/battery-monitoring).

## How it fits

Add `leptoskit_plugin_battery` as a direct dependency and select the `battery` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::Battery` rather than resolving the plugin on every call. Unknown readings stay unknown instead of being reported as a full battery, and a simulator is never presented as physical hardware.

## Use it

```rust,ignore
let battery = leptoskit_plugin_battery::guest::Battery::new()?;
let mut current = battery.snapshot().await?;
loop {
    update_display(current);
    current = battery.wait_for_change(current).await?;
}
```

## Learn more

- [Crate documentation](https://docs.rs/leptoskit_plugin_battery)
- [leptoskit documentation](https://ifiokjr.github.io/leptoskit/)
- [Repository](https://github.com/ifiokjr/leptoskit)
- [Plugin inventory](https://ifiokjr.github.io/leptoskit/platforms/plugin-inventory.html)
