# LeptosKit Magnetometer

Magnetic field in microteslas along native device axes. This is a magnetic sensor measurement, not a compass heading; platform calibration differs.

Select `magnetometer` in the app's native plugin list and add this crate as a direct Cargo dependency. Automatic registration installs the Swift or Kotlin handler plus the shared sensor support package. iOS requires a nonempty `NSMotionUsageDescription`.

The `guest::Magnetometer` client resolves the plugin once. Call `available().await` to query hardware availability, then `next(Interval::new(16)?).await` for one fresh sample. There can be only one outstanding reading per plugin. Dropping the pending call, closing the plugin, or backgrounding the native activity stops acquisition. Repeated calls are bounded but do not form a lossless fixed-rate recording stream.

See the [shared sensor contract](../leptoskit_sensor/readme.md) for exact binary layout, timestamps, cancellation, browser permissions, and platform limits. Native iOS and Android use real system sensor APIs. Desktop sensors and browser pressure are explicitly unavailable. Browser acceleration/rotation use Generic Sensor or DeviceMotion; browser magnetometer needs Generic Sensor support.
