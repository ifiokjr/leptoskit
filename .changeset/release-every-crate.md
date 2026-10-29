---
"release": patch
---

# Release every platform backend with the rest of leptoskit

`leptoskit_uikit`, `leptoskit_android`, `leptoskit_gtk`, `leptoskit_windows`, and `leptoskit_plugin_router` now release alongside the other leptoskit crates. Before, a release raised their versions but left the versions the other crates require of them where they were, so the first minor release would have published crates that could not resolve each other.
