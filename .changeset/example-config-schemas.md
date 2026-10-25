---
"release": patch
---

Pin every checked-in `leptoskit.toml` to `schema_version = 0` and the public v0 JSON Schema so examples, benchmarks, and CLI fixtures have editor autocomplete and diagnostics. Preserve the required `#:schema` directive through TOML formatting and use the same syntax in documentation examples.
