---
"release": minor
---

# Stream motion sensors, and read them all from one plugin

`leptoskit_plugin_sensors`, runtime name `sensors`, is the port of Flutter's `sensors_plus`. One plugin streams the accelerometer, acceleration with gravity removed (new on every platform), the gyroscope, the magnetometer, and the barometer:

```rust
let sensors = Sensors::new()?;
let mut tilt = sensors.accelerometer(Interval::GAME);
while let Ok(sample) = tilt.next().await { /* … */ }
```

The `accelerometer`, `gyroscope`, `magnetometer`, and `barometer` plugins gain the same `watch` stream, and in the browser `web::watch` keeps a Generic Sensor or `devicemotion` listener running. Sensor plugins used to read one sample per call and stop the hardware in between. A stream keeps the sensor running between reads and holds the newest 64 samples. `Watch::dropped` counts any samples your app read too slowly to see, and dropping the stream stops the sensor. `Interval::NORMAL`, `UI`, `GAME`, and `FASTEST` match `sensors_plus`'s presets.

On iOS, every sensor plugin now reads through one shared motion manager, as Apple recommends, so selecting several plugins no longer starts competing CoreMotion feeds.

**Behaviour change:** iOS acceleration now uses the same axis directions as Android and the browser. A phone lying face up reads about +9.8 m/s² on z everywhere; iOS used to report −9.8. If your app compensated for the iOS sign, remove that compensation.
