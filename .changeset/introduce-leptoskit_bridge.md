---
leptoskit_bridge: major
---

# leptoskit_bridge: the plugin transport

`leptoskit_bridge` is the bounded binary transport between a Wasm guest and the native plugins it is allowed to call.

The host registers each plugin and grants its methods, so an over-the-air bundle cannot add native capabilities. Every call is bounded in size and number, is cancelled when the guest drops it, and fails with error codes the Swift, Kotlin, and C++ hosts share. Most apps never name it directly; each runtime plugin wraps it in a typed `guest` module.
