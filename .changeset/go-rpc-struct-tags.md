---
"release": patch
---

# Escape Go RPC struct tags

Encode schema property and parameter names before writing Go struct tags, preventing consumed RPC specifications from injecting Go source through backticks, quotes, or control characters.
