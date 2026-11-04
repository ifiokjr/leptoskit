---
leptoskit_wasm: major
---

# leptoskit_wasm: the on-device Wasm runtime

`leptoskit_wasm` runs the app's interface bundle on the device in `wasmi`, an interpreter that is allowed on iOS where compiling code at runtime is not, and drives a native backend through a versioned ABI.

This is what makes the shipped app a shell: the interface arrives as Wasm, renders through the same renderer, and a code push release can replace it without a store update. Every entry point is fuel-metered and rendering work is capped, so a bad bundle cannot hang the device. The generated iOS, Android, macOS, Windows, Linux, and terminal apps embed it.
