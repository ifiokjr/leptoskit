---
leptoskit_wasm: breaking
---

# leptoskit_wasm: the on-device Wasm runtime

`leptoskit_wasm` runs the app's interface bundle in `wasmi`, drives a native backend through a versioned, fuel-metered ABI, and applies code push releases.

This is what makes the shipped app a shell: the interface arrives as Wasm, renders through the same renderer, and can be replaced without a store update. The generated iOS, Android, macOS, Windows, and Linux hosts embed it.
