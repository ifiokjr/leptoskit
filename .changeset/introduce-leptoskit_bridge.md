---
leptoskit_bridge: breaking
---

# leptoskit_bridge: the plugin transport

`leptoskit_bridge` is the bounded binary transport between a Wasm guest and the native plugins it is allowed to call.

Plugins define methods, payload limits, cancellation, and error codes once; the guest and the Swift, Kotlin, or C++ host then speak the same protocol. Most apps never name it directly, but every runtime plugin does.
