<h1 align="center">leptoskit_plugin_network</h1>

<p align="center">
  <strong>leptoskit_plugin_network</strong> is the leptoskit plugin for network state and demand-driven change observation.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://docs.rs/leptoskit_plugin_network"><strong>Docs</strong></a> ·
  <a href="https://github.com/ifiokjr/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<p align="center">
  <a href="https://crates.io/crates/leptoskit_plugin_network"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_plugin_network.svg"></a>
  <a href="https://docs.rs/leptoskit_plugin_network"><img alt="docs.rs" src="https://docs.rs/leptoskit_plugin_network/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<br>

## Overview

Network state and demand-driven change observation for native and browser apps. Each state is a strict 32-byte binary message; the cached guest client resolves its plugin identifier once. Selecting `network` in `native.plugins` registers the Swift/Kotlin implementation automatically. Android adds the normal `android.permission.ACCESS_NETWORK_STATE` permission; no runtime prompt is needed.

- iOS/macOS: `NWPathMonitor` reports route availability, transport, expensive connections, and low-data mode. A monitor starts per request, waits for its first actual callback, and is cancelled on completion, cancellation, or close.
- Android: `ConnectivityManager` reports the default network, transport, metering, and OS-validated internet capability. Change callbacks register before sampling and unregister on completion, cancellation, or close.
- Desktop Rust: `desktop::interfaces()` exposes local interface addresses and operational status on macOS, Linux, and Windows. `desktop::snapshot()` reports whether a non-loopback interface is operational. It does not infer transport from interface names or claim that an interface establishes a default route.
- Browser: `navigator.onLine` supplies an online hint; supported `NetworkInformation` properties supply estimated throughput, latency, performance class, and save-data preferences. Missing properties remain unknown. Browser change listeners are removed when the waiting future ends or is dropped.

`guest::Network::wait_for_change(previous)` returns the first different latest snapshot. Reissue it with that snapshot to continue observing. Intermediate states can coalesce; this is not a transition log. Mobile/Apple handlers permit one outstanding observer; the Apple handler also returns `Busy` for a snapshot while another request owns its monitor. Dropping the guest call cancels native work through the bridge. The synchronous desktop Rust adapter supports snapshots only; desktop applications can choose their own refresh schedule or use the Apple native monitor on macOS.

An active connection never proves that an application server is reachable. `internet_validated` is Android's OS assessment; on other platforms it remains unknown. Browser online hints can be wrong. No connectivity probe or third-party public-IP request occurs. This port does not yet expose Expo's airplane-mode query, SSID/MAC access, or a cross-platform local-IP RPC.

Reference APIs: [Expo Network](https://docs.expo.dev/versions/latest/sdk/network/), [Apple NWPathMonitor](https://developer.apple.com/documentation/network/nwpathmonitor), [Android network capabilities](https://developer.android.com/reference/android/net/NetworkCapabilities), [MDN navigator.onLine](https://developer.mozilla.org/en-US/docs/Web/API/Navigator/onLine), [MDN NetworkInformation](https://developer.mozilla.org/en-US/docs/Web/API/NetworkInformation). Tauri's [plugin directory](https://tauri.app/plugin/) does not define an official network-state API to mirror; this crate exposes the operating systems' capabilities without adding a desktop webview dependency.

## How it fits

Add `leptoskit_plugin_network` as a direct dependency and select the `network` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::Network` rather than resolving the plugin on every call. Apple uses `NWPathMonitor`, Android uses `ConnectivityManager`, desktop reports interface state, and the browser uses `navigator.onLine` plus `NetworkInformation`; an active connection never claims a server is reachable.

## Use it

```toml
# In a native host, select `network` in `native.plugins`.
[dependencies]
leptoskit_plugin_network = { version = "0.1", features = ["guest"] }
```

## Learn more

- [Crate documentation](https://docs.rs/leptoskit_plugin_network)
- [leptoskit documentation](https://ifiokjr.github.io/leptoskit/)
- [Repository](https://github.com/ifiokjr/leptoskit)
- [Plugin inventory](https://ifiokjr.github.io/leptoskit/platforms/plugin-inventory.html)
