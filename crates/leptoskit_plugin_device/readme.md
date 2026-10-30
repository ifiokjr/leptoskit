# leptoskit_plugin_device

Device information for native LeptosKit guests, Rust desktop hosts, and browsers. Each snapshot uses a validated binary response. Guest clients resolve the plugin once and reuse its numeric ID. No persistent identifiers or user-assigned device names are read.

```rust,ignore
let device = leptoskit_plugin_device::guest::Device::new()?;
let snapshot = device.snapshot().await?;
```

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
