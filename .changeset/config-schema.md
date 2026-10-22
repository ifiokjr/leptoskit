---
"release": minor
---

# Add editor schemas and explicit configuration migrations

Generated `leptoskit.toml` files include a pinned JSON Schema directive and `schema_version = 1` for editor autocomplete and diagnostics. The docs publish `schema.json`, `schemas/leptoskit.schema.json`, and the major-pinned `schemas/leptoskit.v1.schema.json`, generated from Rust types and checked for drift in CI.

Run `leptoskit migrate --dry-run` to preview an existing file, `--check` to detect pending migration, or `leptoskit migrate` to apply it. The first migration stamps unversioned files and moves `build.wasm_opt` to `bundle.wasm_opt` while preserving comments. Invalid settings, conflicting keys, and future versions fail without changing the file. Existing unversioned configurations remain readable.
