---
leptoskit_sensor: breaking
---

# leptoskit_sensor: the shared sensor protocol

`leptoskit_sensor` is the shared protocol and native acquisition lifecycle behind the accelerometer, gyroscope, magnetometer, barometer, light sensor, and pedometer plugins.

The sensor plugins share its request/reply layout, cancellation rules, timestamps, and browser fallbacks, so an app can switch sensors without relearning the contract. Apps depend on the individual sensor crate; this crate is the common floor.
