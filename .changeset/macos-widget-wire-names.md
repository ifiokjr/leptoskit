---
"release": patch
---

# Render canonical widget names in generated macOS hosts

The AppKit host accepts the canonical widget names serialized by the Rust renderer and retains legacy aliases, restoring the generated application's controls after the shared component update.
