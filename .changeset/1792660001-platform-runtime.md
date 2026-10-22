---
"release": minor
---

# Share application components across platform builds

Add a native Wasm guest adapter for portable component trees and explicit iOS, Android, macOS, Windows, and Linux feature markers. Native SDK dependencies follow the actual compilation target, so destination features remain usable by Wasm applications.

The new `leptoskit_build` helper and `platform_module!` macro select platform source overrides with a shared fallback, track file additions and removals, and reject conflicting destination features.
