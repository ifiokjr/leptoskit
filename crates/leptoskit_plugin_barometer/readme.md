<h1 align="center">leptoskit_plugin_barometer</h1>

<p align="center">
  <strong>leptoskit_plugin_barometer</strong> is the leptoskit plugin for cancellable atmospheric pressure readings in hectopascals along native device axes.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://docs.rs/leptoskit_plugin_barometer"><strong>Docs</strong></a> ·
  <a href="https://github.com/leptoskit/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<!-- {=crateBadges|replace:"CRATE_NAME":"leptoskit_plugin_barometer"} -->

<p align="center">
  <a href="https://crates.io/crates/leptoskit_plugin_barometer"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_plugin_barometer.svg"></a>
  <a href="https://docs.rs/leptoskit_plugin_barometer"><img alt="docs.rs" src="https://docs.rs/leptoskit_plugin_barometer/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<!-- {/crateBadges} -->

<br>

## Overview

Atmospheric pressure in hectopascals (millibars). Apple kilopascals are converted to hectopascals. No relative altitude estimate is fabricated.

Select `barometer` in the app's native plugin list and add this crate as a direct Cargo dependency. Automatic registration installs the Swift or Kotlin handler plus the shared sensor support package. iOS requires a nonempty `NSMotionUsageDescription`.

The `guest::Barometer` client resolves the plugin once. Call `available().await` to query hardware availability. `watch(Interval::GAME)` returns a stream: await `next()` in a loop and drop it to stop the sensor. `next(Interval::new(16)?).await` waits for one fresh sample instead. One read and one stream can be outstanding per plugin. Closing the plugin stops acquisition; backgrounding the app ends a pending read, while a stream waits for the app to return. A stream keeps the newest 64 samples between reads and counts the rest in `dropped()`, so it is bounded rather than a lossless recording. For several sensors at once, the [`sensors`](../leptoskit_plugin_sensors/readme.md) plugin streams them all and shares this implementation.

See the [shared sensor contract](../leptoskit_sensor/readme.md) for exact binary layout, timestamps, cancellation, browser permissions, and platform limits. Native iOS and Android use real system sensor APIs. Desktop sensors and browser pressure are explicitly unavailable. Browser acceleration/rotation use Generic Sensor or DeviceMotion; browser magnetometer needs Generic Sensor support.

## How it fits

<!-- {=p_barometerSummary} -->

Add `leptoskit_plugin_barometer` as a direct dependency and select the `barometer` plugin in `native.plugins`; supported generated hosts discover it from Cargo metadata. Generated Windows, Linux, and terminal hosts currently omit runtime plugins; use explicit registration in a custom host there. Guests enable the `guest` feature and reuse `guest::Barometer` rather than resolving the plugin on every call. iOS and Android use real pressure sensors, desktop has no sensor, and no relative altitude estimate is fabricated.

<!-- {/p_barometerSummary} -->

## Use it

```rust,ignore
use leptoskit_plugin_barometer::guest::Barometer;
use leptoskit_sensor::Interval;

let sensor = Barometer::new()?;
let sample = sensor.next(Interval::new(16)?).await?;
```

## Learn more

- [Crate documentation](https://docs.rs/leptoskit_plugin_barometer)
- [leptoskit documentation](https://leptoskit.dev/docs/)
- [Repository](https://github.com/leptoskit/leptoskit)
- [Plugin inventory](https://leptoskit.dev/docs/platforms/plugin-inventory.html)
