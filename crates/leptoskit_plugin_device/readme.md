<h1 align="center">leptoskit_plugin_device</h1>

<p align="center">
  <strong>leptoskit_plugin_device</strong> is the leptoskit plugin for device and operating system information without persistent identifiers or user-assigned names.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://docs.rs/leptoskit_plugin_device"><strong>Docs</strong></a> ·
  <a href="https://github.com/ifiokjr/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<p align="center">
  <a href="https://crates.io/crates/leptoskit_plugin_device"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_plugin_device.svg"></a>
  <a href="https://docs.rs/leptoskit_plugin_device"><img alt="docs.rs" src="https://docs.rs/leptoskit_plugin_device/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<br>

## Overview

Device information for native LeptosKit guests, Rust desktop hosts, and browsers. Each snapshot uses a validated binary response. Guest clients resolve the plugin once and reuse its numeric ID. No persistent identifiers or user-assigned device names are read.

Add this crate as a direct app dependency and `"device"` to `native.plugins` to register the UIKit or Android implementation automatically. Rust desktop hosts register `desktop::Device`; browser apps call `web::snapshot()` directly.

| Platform                     | Information                                                                                                                    |
| ---------------------------- | ------------------------------------------------------------------------------------------------------------------------------ |
| iOS                          | Apple manufacturer/brand, product family, OS/version, native plugin architecture, OS memory, interface idiom, simulator status |
| Android                      | Build manufacturer/brand/model, OS/version, preferred ABI, OS memory, UI mode/screen width, recognized emulator signature      |
| macOS, Windows, Linux (Rust) | OS/version, architecture, OS memory; hardware vendor/model and virtualization status stay unknown                              |
| Browser                      | Low-entropy User-Agent Client Hints OS platform when available; remaining fields stay unknown                                  |

Android phone/tablet classification follows the configured smallest screen width (600 dp threshold), so resizing and desktop modes can change that classification. Android reports `simulator: Some(true)` for the conventional `goldfish`/`ranchu` emulator hardware signatures and `None` otherwise. This is informational, not attestation. iOS simulator detection is a compile-time platform signal. Desktop virtualization is not inferred.

Memory means total bytes exposed to the operating system, not the application's memory budget. Browser `deviceMemory` is coarse and privacy-reduced, so it is not used as exact RAM. Browser hints may be unavailable or reduced; missing fields are explicit `None` values. No user-agent parsing or high-entropy hint requests run. Desktop reads refresh RAM only and do not enumerate processes or sample CPUs.

This ports the basic information surface of [Expo Device](https://docs.expo.dev/versions/latest/sdk/device/). Root detection, sideload settings, maximum heap size, and CPU feature enumeration are not implemented. The desktop implementation uses [sysinfo 0.35](https://docs.rs/sysinfo/0.35.2/sysinfo/); only its system API and App Store sandbox compatibility features are enabled.

## How it fits

Add `leptoskit_plugin_device` as a direct dependency and select the `device` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::Device` rather than resolving the plugin on every call. Native guests, Rust desktop hosts, and browsers each return a validated binary snapshot with the fields that platform can actually provide.

## Use it

```rust,ignore
let device = leptoskit_plugin_device::guest::Device::new()?;
let snapshot = device.snapshot().await?;
```

## Learn more

- [Crate documentation](https://docs.rs/leptoskit_plugin_device)
- [leptoskit documentation](https://ifiokjr.github.io/leptoskit/)
- [Repository](https://github.com/ifiokjr/leptoskit)
- [Plugin inventory](https://ifiokjr.github.io/leptoskit/platforms/plugin-inventory.html)
