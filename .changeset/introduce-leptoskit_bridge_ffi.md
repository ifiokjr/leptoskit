---
leptoskit_bridge_ffi: breaking
---

# leptoskit_bridge_ffi: the C ABI for native shells

`leptoskit_bridge_ffi` is the C ABI and the Swift, Kotlin, and C++ glue that let a native app shell host a leptoskit Wasm guest and forward its plugin calls.

Generated macOS, iOS, Android, and HarmonyOS shells link against it; the Rust backends are used by tests and by hosts that embed leptoskit directly. It is the boundary between the platform's UI thread and the guest.
