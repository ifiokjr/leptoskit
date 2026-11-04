---
leptoskit_sensor: major
---

# leptoskit_sensor: the shared sensor protocol

`leptoskit_sensor` is the shared protocol and native lifecycle behind the accelerometer, gyroscope, magnetometer, barometer, light sensor, and pedometer plugins.

Each request takes one fresh reading and then releases the sensor, so repeated calls are not a fixed-rate recording stream; timestamps come from the platform's monotonic clock, not UTC. The sensor plugins share its request and reply layout, cancellation rules, and browser adapters over the Generic Sensor API and device motion events, so an app can switch sensors without relearning the contract. Apps depend on the individual sensor crate; this crate is the common floor.
