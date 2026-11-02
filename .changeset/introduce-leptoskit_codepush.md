---
leptoskit_codepush: breaking
---

# leptoskit_codepush: over-the-air updates

`leptoskit_codepush` provides signed, content-hashed over-the-air releases for Wasm bundles, with a single-file container, binary deltas, a static channel layout, and a client that verifies every byte.

`leptoskit codepush` uses it to pack and publish, `leptoskit_axum` serves it, and `leptoskit_wasm` consumes it on the device. It is the React Native update model applied to the interface bundle.
