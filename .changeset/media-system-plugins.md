---
"release": minor
---

# Add image transformation and capture-policy plugins

Add a separate image manipulation crate with bounded JPEG/PNG decoding, ordered crop, resize, rotation and flip operations, and encoding. Native hosts run transforms on a Rust worker; browser applications can call the same engine. Input, output, dimensions and transformation work have explicit limits. Decoder-private allocations remain subject to codec behavior. This crate requires Rust 1.88.

Add a screen capture crate with Android window-owned secure-flag leases, Android 14 screenshot notifications, iOS scene recording-state observation and app-wide screenshot notifications. Cancellation and teardown release subscriptions and owned leases. Capture prevention is unavailable on iOS; desktop and browser capture capabilities are unavailable. The plugin never captures pixels.
