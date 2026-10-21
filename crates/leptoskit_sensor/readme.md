# LeptosKit sensor support

Shared protocol, browser implementation, and native acquisition lifecycle for the independently selected `accelerometer`, `gyroscope`, `magnetometer`, `barometer`, `light_sensor`, and `pedometer` plugin crates. This support crate is not registered as a plugin. Its `native-support` Cargo metadata makes the CLI copy its Swift package and Kotlin sources when a selected plugin depends on it.

## Contract

Method 0 has an empty request and returns a four-byte little-endian boolean indicating hardware/API availability. Permission is checked separately when sampling. Method 1 accepts one little-endian `u32` interval in milliseconds, bounded to 5–60,000. Android and CoreMotion motion sensors accept this requested rate; barometer and browser DeviceMotion timing is OS-controlled. No high-sampling-rate Android permission is required.

Vector results are four little-endian finite `f64`s: monotonic timestamp in seconds, x, y, z. Pressure results contain timestamp and positive pressure in hectopascals. These clocks are platform-local, not UTC, and must not be compared across devices or APIs. Coordinate axes stay in each platform's native device frame; screen rotation does not remap them. Apple acceleration in units of gravity is converted using 9.80665 m/s², and Apple pressure in kPa is converted to hPa. Browser DeviceMotion rotation is converted from degrees/s to radians/s (beta → x, gamma → y, alpha → z).

Each call waits for a fresh sample, stops acquisition before returning, and permits only one outstanding waiter per plugin. Cancellation, plugin close, and native activity backgrounding release registrations. Continuous callers must await each result before requesting another. This bounds memory and avoids an unbounded event queue; it is not a lossless fixed-rate recording API. Hardware that stops emitting can keep a request pending until cancelled. Apply an application timeout where needed.

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

Rust tests cover wire lengths, finite-value validation, interval bounds, SI units, signed vector components, and absent capabilities. Swift tests exercise the real bridge-owned payload path and unavailable/closed/malformed behavior. Android instrumentation uses real SensorManager to test acquisition, units on the stationary CI emulator, concurrent waiter rejection, cancellation, and close. Physical hardware and browser sensor permissions require device testing beyond these checks.

References: [Apple CoreMotion](https://developer.apple.com/documentation/coremotion/cmmotionmanager), [Android SensorEvent](https://developer.android.com/reference/android/hardware/SensorEvent), [Generic Sensor standard](https://www.w3.org/TR/generic-sensor/).
