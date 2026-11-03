---
"release": patch
---

# Report missing desktop pedometer hardware

The Apple pedometer plugin now answers a valid availability query with `false` on macOS and validates malformed requests. Permission and step-reading calls remain unavailable, and closing the plugin still cancels later calls.
