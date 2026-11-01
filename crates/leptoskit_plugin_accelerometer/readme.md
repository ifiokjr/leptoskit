<h1 align="center">leptoskit_plugin_accelerometer</h1>

<p align="center">
  <strong>leptoskit_plugin_accelerometer</strong> is the leptoskit plugin for cancellable acceleration readings including gravity, in metres per second squared, along native device axes.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://docs.rs/leptoskit_plugin_accelerometer"><strong>Docs</strong></a> ·
  <a href="https://github.com/ifiokjr/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<p align="center">
  <a href="https://crates.io/crates/leptoskit_plugin_accelerometer"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_plugin_accelerometer.svg"></a>
  <a href="https://docs.rs/leptoskit_plugin_accelerometer"><img alt="docs.rs" src="https://docs.rs/leptoskit_plugin_accelerometer/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<br>

## Overview

Acceleration including gravity in metres per second squared. Apple gravity units are converted to SI. Native axes are preserved.

Select `accelerometer` in the app's native plugin list and add this crate as a direct Cargo dependency. Automatic registration installs the Swift or Kotlin handler plus the shared sensor support package. iOS requires a nonempty `NSMotionUsageDescription`.

The `guest::Accelerometer` client resolves the plugin once. Call `available().await` to query hardware availability, then `next(Interval::new(16)?).await` for one fresh sample. There can be only one outstanding reading per plugin. Dropping the pending call, closing the plugin, or backgrounding the native activity stops acquisition. Repeated calls are bounded but do not form a lossless fixed-rate recording stream.

See the [shared sensor contract](../leptoskit_sensor/readme.md) for exact binary layout, timestamps, cancellation, browser permissions, and platform limits. Native iOS and Android use real system sensor APIs. Desktop sensors and browser pressure are explicitly unavailable. Browser acceleration/rotation use Generic Sensor or DeviceMotion; browser magnetometer needs Generic Sensor support.

## How it fits

Add `leptoskit_plugin_accelerometer` as a direct dependency and select the `accelerometer` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::Accelerometer` rather than resolving the plugin on every call. iOS and Android use real system sensors, desktop has no sensor, and the browser uses Generic Sensor or DeviceMotion.

## Use it

```rust,ignore
use leptoskit_plugin_accelerometer::guest::Accelerometer;
use leptoskit_sensor::Interval;

let sensor = Accelerometer::new()?;
let sample = sensor.next(Interval::new(16)?).await?;
```

## Learn more

- [Crate documentation](https://docs.rs/leptoskit_plugin_accelerometer)
- [leptoskit documentation](https://ifiokjr.github.io/leptoskit/)
- [Repository](https://github.com/ifiokjr/leptoskit)
- [Plugin inventory](https://ifiokjr.github.io/leptoskit/platforms/plugin-inventory.html)
