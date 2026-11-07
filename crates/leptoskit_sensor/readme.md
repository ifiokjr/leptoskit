<h1 align="center">leptoskit_sensor</h1>

<p align="center">
  <strong>leptoskit_sensor</strong> is the shared protocol and native acquisition lifecycle behind the accelerometer, gyroscope, magnetometer, barometer, light sensor, and pedometer plugins.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://docs.rs/leptoskit_sensor"><strong>Docs</strong></a> ·
  <a href="https://github.com/leptoskit/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<!-- {=crateBadges|replace:"CRATE_NAME":"leptoskit_sensor"} -->

<p align="center">
  <a href="https://crates.io/crates/leptoskit_sensor"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_sensor.svg"></a>
  <a href="https://docs.rs/leptoskit_sensor"><img alt="docs.rs" src="https://docs.rs/leptoskit_sensor/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<!-- {/crateBadges} -->

<br>

## Overview

Shared protocol, browser implementation, and native acquisition lifecycle for the combined `sensors` plugin and the independently selected `accelerometer`, `gyroscope`, `magnetometer`, `barometer`, `light_sensor`, and `pedometer` plugin crates. This support crate is not registered as a plugin. Its `native-support` Cargo metadata makes the CLI copy its Swift package and Kotlin sources when a selected plugin depends on it.

## Contract

Method 0 has an empty request and returns a four-byte little-endian boolean indicating hardware/API availability. Permission is checked separately when sampling. Method 1 accepts one little-endian `u32` interval in milliseconds, bounded to 5–60,000. Android and CoreMotion motion sensors accept this requested rate; barometer and browser DeviceMotion timing is OS-controlled. No high-sampling-rate Android permission is required.

Vector results are four little-endian finite `f64`s: monotonic timestamp in seconds, x, y, z. Pressure results contain timestamp and positive pressure in hectopascals. These clocks are platform-local, not UTC, and must not be compared across devices or APIs. Coordinates are device axes, which screen rotation does not remap, and every platform uses Android's and the W3C's directions: a phone lying face up reads about +9.8 m/s² on z. Apple acceleration in units of gravity is multiplied by −9.80665 m/s² to match, as Flutter's `sensors_plus` does; Apple rotation and magnetic axes already agree. Apple pressure in kPa is converted to hPa. Browser DeviceMotion rotation is converted from degrees/s to radians/s (beta → x, gamma → y, alpha → z).

Each method 1 call waits for a fresh sample, stops acquisition before returning, and permits only one outstanding waiter per plugin. Cancellation, plugin close, and app backgrounding release registrations. Hardware that stops emitting can keep a request pending until cancelled. Apply an application timeout where needed.

## Streams

Method 2 (`WATCH`) streams. Its request is the `u32` interval followed by a `u64` cursor: the sequence number of the newest sample the caller has, or zero to start from the next fresh one. The reply is a `u64` cursor (the newest sample's number), a `u32` count of samples dropped before the caller asked, a `u32` sample count from 1 to 64, then that many samples in the method 1 layout, oldest first. A cursor the plugin never issued is refused with InvalidArgument.

Between requests the sensor keeps running and the newest 64 samples wait in a ring, so a caller that asks again promptly sees every sample. After a reply the plugin waits for the next request for a grace period of two seconds, or two intervals if longer, then stops the sensor. Cancelling a pending request stops it at once; that is how `guest::Watch` stops when dropped. A dropped `Watch::next` future does not cancel: the pending request belongs to the watch, so `select!` loses nothing. One stream request per plugin and kind can be outstanding alongside one method 1 read. When the app is backgrounded the hardware stops and a pending stream request keeps waiting; it resumes when the app returns. Light and pedometer do not stream.

The combined `sensors` plugin prefixes every request with a `u32` kind (0 accelerometer, 1 gravity-free acceleration, 2 gyroscope, 3 magnetometer, 4 barometer) and otherwise uses these layouts. On iOS every sensor plugin reads through one shared CoreMotion manager, and a feed runs at the fastest interval any reader asked for. Android registers one listener per reader, which SensorManager supports directly.

## Bounded light and step acquisition

Light adds an explicit 1–120000 ms deadline after the interval (`u32`, `u32`). Its reply is a `u32` zero followed by two finite nonnegative `f64` values (monotonic seconds, lux), or a `u32` one alone for timeout. These tagged replies are separate from the original four sensors' unchanged wire format. Android reuses the same acquisition/cleanup implementation; browser AmbientLightSensor reuses the Generic Sensor lifecycle. Activity-backed bounded acquisition requires visible activity content, not window focus, and otherwise returns Unavailable.

Pedometer has its own public protocol and permission/history methods. Its Android backend uses the same bounded SensorManager acquisition but emits a distinct boot-counter scope; iOS uses CMPedometer interval counts. See the public crate readme for precise clock, permission, history, float precision, and cancellation limits. Neither helper nor public plugin claims that a counter is a daily total.

## Permissions and limits

- iOS apps must provide a nonempty `NSMotionUsageDescription`. Missing usage descriptions and denied/restricted barometer permissions return PermissionDenied. CoreMotion reports unavailable hardware explicitly. Physical sensor behavior cannot be proved by an iOS simulator.
- Android uses SensorManager and at most 200 Hz. No motion runtime permission is required for these four sensors at this rate. Cancelled registrations are removed on the main thread.
- Browser acceleration and rotation use Generic Sensor where present, with DeviceMotion fallback. Magnetometer requires Generic Sensor. HTTPS/secure context and applicable Permissions-Policy are required. Call `request_permission` from a click/tap handler for Safari, then request readings. A denied gesture-gated permission is never silently bypassed. Browser API presence does not prove physical hardware availability.
- Browser barometers and desktop sensor backends are unavailable; no estimated or zero-valued readings are substituted. macOS Swift handlers report unavailable.
- Magnetic readings are the platform's sensor values, not a geographic compass heading. Hardware calibration/accuracy varies.

## Verification

Rust tests cover wire lengths, finite-value validation, interval bounds, SI units, signed vector components, stream requests and batches, and absent capabilities. Swift tests exercise the real bridge-owned payload path and unavailable/closed/malformed behavior, and drive the shared feed with a stand-in motion source to cover interval negotiation between plugins, one-shot reads, cursors, ring overflow, the grace period, cancellation, app suspension, and kind routing. Android instrumentation uses real SensorManager to test acquisition, units on the stationary CI emulator, concurrent waiter rejection, cancellation, close, streams after a cursor, the grace period, and the combined plugin's routing. Browser tests check stream order, the bounded queue, error delivery, and that dropping a stream stops the sensor. Physical hardware and browser sensor permissions require device testing beyond these checks.

References: [Apple CoreMotion](https://developer.apple.com/documentation/coremotion/cmmotionmanager), [Android SensorEvent](https://developer.android.com/reference/android/hardware/SensorEvent), [Generic Sensor standard](https://www.w3.org/TR/generic-sensor/).

## How it fits

<!-- {=sensorSummary} -->

The sensor plugins share its request/reply layout, cancellation rules, timestamps, and browser fallbacks, so an app can switch sensors without relearning the contract. Apps depend on the individual sensor crate; this crate is the common floor.

<!-- {/sensorSummary} -->

## Use it

```rust,ignore
// Each sensor crate re-exports the shared Interval and sample types.
use leptoskit_sensor::Interval;
```

## Learn more

- [Crate documentation](https://docs.rs/leptoskit_sensor)
- [leptoskit documentation](https://leptoskit.dev/docs/)
- [Repository](https://github.com/leptoskit/leptoskit)
- [Controlling devices](https://leptoskit.dev/docs/guide/19-controlling-devices.html)
