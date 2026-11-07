<h1 align="center">leptoskit_plugin_sensors</h1>

<p align="center">
  <strong>leptoskit_plugin_sensors</strong> is the leptoskit plugin that streams the accelerometer, gravity-free acceleration, gyroscope, magnetometer, and barometer from one plugin.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://docs.rs/leptoskit_plugin_sensors"><strong>Docs</strong></a> ·
  <a href="https://github.com/leptoskit/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<!-- {=crateBadges|replace:"CRATE_NAME":"leptoskit_plugin_sensors"} -->

<p align="center">
  <a href="https://crates.io/crates/leptoskit_plugin_sensors"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_plugin_sensors.svg"></a>
  <a href="https://docs.rs/leptoskit_plugin_sensors"><img alt="docs.rs" src="https://docs.rs/leptoskit_plugin_sensors/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<!-- {/crateBadges} -->

<br>

## Overview

Motion-driven interfaces need a steady series of samples, not one reading at a time: a tilt control, a shake gesture, a compass needle, a step-free altimeter readout. This plugin covers what Flutter's `sensors_plus` covers, behind one plugin name:

| Kind                      | Unit  | iOS                      | Android                    | Browser                                       |
| ------------------------- | ----- | ------------------------ | -------------------------- | --------------------------------------------- |
| `Kind::Accelerometer`     | m/s²  | CoreMotion accelerometer | `TYPE_ACCELEROMETER`       | `Accelerometer`, or DeviceMotion with gravity |
| `Kind::UserAccelerometer` | m/s²  | CoreMotion device motion | `TYPE_LINEAR_ACCELERATION` | `LinearAccelerationSensor`, or DeviceMotion   |
| `Kind::Gyroscope`         | rad/s | CoreMotion gyroscope     | `TYPE_GYROSCOPE`           | `Gyroscope`, or DeviceMotion rotation rate    |
| `Kind::Magnetometer`      | µT    | CoreMotion magnetometer  | `TYPE_MAGNETIC_FIELD`      | `Magnetometer` where the browser has it       |
| `Kind::Barometer`         | hPa   | CoreMotion altimeter     | `TYPE_PRESSURE`            | Unavailable                                   |

Every platform reports the same axes. A phone lying face up reads about +9.8 m/s² on z: CoreMotion's units of gravity are scaled to m/s² and negated to match Android and the W3C sensors. macOS, Windows, and Linux have no app-readable motion sensors, so every kind reports unavailable there instead of inventing readings.

Each kind streams independently. A stream keeps the newest 64 samples on the native side; read it often enough and you see every sample, fall behind and `Watch::dropped` counts what you missed. Dropping the stream stops the sensor. This plugin shares its implementation with the single-sensor `accelerometer`, `gyroscope`, `magnetometer`, and `barometer` plugins, and on iOS every one of them reads through one motion manager, so selecting several costs nothing extra.

iOS apps that read the barometer need a nonempty `NSMotionUsageDescription`. Android needs no runtime permission at these rates. Browsers need a secure context; Safari asks for motion permission, so start a browser stream from a click or tap.

See the [shared sensor contract](../leptoskit_sensor/readme.md) for the wire format, clocks, and cancellation rules.

## How it fits

<!-- {=p_sensorsSummary} -->

Add `leptoskit_plugin_sensors` as a direct dependency and select the `sensors` plugin in `native.plugins`; supported generated hosts discover it from Cargo metadata. Generated Windows, Linux, and terminal hosts currently omit runtime plugins; use explicit registration in a custom host there. Guests enable the `guest` feature and reuse `guest::Sensors`, which streams the accelerometer, gravity-free acceleration, gyroscope, magnetometer, and barometer from one plugin. iOS and Android use real system sensors on the same axes, desktop has no sensor, and the browser uses Generic Sensor or `DeviceMotion` and has no barometer.

<!-- {/p_sensorsSummary} -->

## Use it

```rust,ignore
use leptoskit_plugin_sensors::Interval;
use leptoskit_plugin_sensors::guest::Sensors;

let sensors = Sensors::new()?;
let mut tilt = sensors.accelerometer(Interval::GAME);

loop {
	let sample = tilt.next().await?;
	set_tilt(sample.x, sample.y);
}
```

`Interval::NORMAL` (5 Hz), `UI` (about 15 Hz), `GAME` (50 Hz), and `FASTEST` (200 Hz) match `sensors_plus`'s presets; the platform may deliver more slowly. Use `read_accelerometer` and the other `read_*` methods for one fresh sample.

## Learn more

- [Crate documentation](https://docs.rs/leptoskit_plugin_sensors)
- [leptoskit documentation](https://leptoskit.dev/docs/)
- [Repository](https://github.com/leptoskit/leptoskit)
- [Plugin inventory](https://leptoskit.dev/docs/platforms/plugin-inventory.html)
