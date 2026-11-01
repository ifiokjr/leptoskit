<h1 align="center">leptoskit_plugin_light_sensor</h1>

<p align="center">
  <strong>leptoskit_plugin_light_sensor</strong> is the leptoskit plugin for ambient illuminance in lux from Android's light sensor or the browser's AmbientLightSensor.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://docs.rs/leptoskit_plugin_light_sensor"><strong>Docs</strong></a> ·
  <a href="https://github.com/ifiokjr/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<p align="center">
  <a href="https://crates.io/crates/leptoskit_plugin_light_sensor"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_plugin_light_sensor.svg"></a>
  <a href="https://docs.rs/leptoskit_plugin_light_sensor"><img alt="docs.rs" src="https://docs.rs/leptoskit_plugin_light_sensor/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<br>

## Overview

Android `Sensor.TYPE_LIGHT` reports ambient illuminance in lux. Browsers with the experimental `AmbientLightSensor` API use that API in a secure context; it may be absent, policy blocked, or rounded for privacy. iOS/macOS expose no supported public ambient-light API, and desktop Rust has no backend. Availability describes hardware/API presence, not permission or a guaranteed reading. Display brightness is never substituted for illumination.

`next(interval, timeout)` waits for one fresh sample, then unregisters. Timeout is 1–120000 ms; interval is a 5–60000 ms scheduling hint. Android Activity-backed acquisition requires the activity's content to be visible; it need not have focus. One outstanding call per plugin; overlap returns Busy. Drop/cancel, native close, and Android activity pause unregister the listener. The browser sensor owns its listeners through RAII; timeout also stops it. Zero lux is valid; missing data is an error. Timestamps are monotonic platform seconds, not Unix time or portable across APIs. Repeated calls have gaps and are not lossless recording.

Select Cargo crate `leptoskit_plugin_light_sensor` and runtime plugin `light_sensor`. Native registration and shared sensor sources are discovered from Cargo metadata. No Android permission is required. Browser `ambient-light-sensor` Permissions Policy and browser authorization apply when starting the sensor; call from an intentional user action.

Physical hardware behavior and accuracy have not been verified. Tests use synthetic values and mocked browser sensors.

References: [Expo LightSensor](https://docs.expo.dev/versions/latest/sdk/light-sensor/), [Android environmental sensors](https://developer.android.com/develop/sensors-and-location/sensors/sensors_environment), [W3C Ambient Light Sensor](https://www.w3.org/TR/ambient-light/).

## How it fits

Add `leptoskit_plugin_light_sensor` as a direct dependency and select the `light_sensor` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::LightSensor` rather than resolving the plugin on every call. iOS, macOS, and desktop have no supported public ambient-light API and return `Unavailable`; display brightness is never substituted for illumination.

## Use it

```rust,ignore
use leptoskit_plugin_light_sensor::guest::LightSensor;
use leptoskit_sensor::Interval;

let sensor = LightSensor::new()?;
let sample = sensor.next(Interval::new(1000)?, 5000).await?;
```

## Learn more

- [Crate documentation](https://docs.rs/leptoskit_plugin_light_sensor)
- [leptoskit documentation](https://ifiokjr.github.io/leptoskit/)
- [Repository](https://github.com/ifiokjr/leptoskit)
- [Plugin inventory](https://ifiokjr.github.io/leptoskit/platforms/plugin-inventory.html)
