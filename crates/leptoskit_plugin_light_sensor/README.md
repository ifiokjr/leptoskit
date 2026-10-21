# Light sensor

Android `Sensor.TYPE_LIGHT` reports ambient illuminance in lux. Browsers with the experimental `AmbientLightSensor` API use that API in a secure context; it may be absent, policy blocked, or rounded for privacy. iOS/macOS expose no supported public ambient-light API, and desktop Rust has no backend. Availability describes hardware/API presence, not permission or a guaranteed reading. Display brightness is never substituted for illumination.

`next(interval, timeout)` waits for one fresh sample, then unregisters. Timeout is 1–120000 ms; interval is a 5–60000 ms scheduling hint. Android Activity-backed acquisition requires the activity's content to be visible; it need not have focus. One outstanding call per plugin; overlap returns Busy. Drop/cancel, native close, and Android activity pause unregister the listener. The browser sensor owns its listeners through RAII; timeout also stops it. Zero lux is valid; missing data is an error. Timestamps are monotonic platform seconds, not Unix time or portable across APIs. Repeated calls have gaps and are not lossless recording.

Select Cargo crate `leptoskit_plugin_light_sensor` and runtime plugin `light_sensor`. Native registration and shared sensor sources are discovered from Cargo metadata. No Android permission is required. Browser `ambient-light-sensor` Permissions Policy and browser authorization apply when starting the sensor; call from an intentional user action.

Physical hardware behavior and accuracy have not been verified. Tests use synthetic values and mocked browser sensors.

References: [Expo LightSensor](https://docs.expo.dev/versions/latest/sdk/light-sensor/), [Android environmental sensors](https://developer.android.com/develop/sensors-and-location/sensors/sensors_environment), [W3C Ambient Light Sensor](https://www.w3.org/TR/ambient-light/).
