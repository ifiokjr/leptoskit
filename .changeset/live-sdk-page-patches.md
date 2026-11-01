---
"leptoskit_rpc_codegen": patch
---

# Keep generated live lists and pages in sync

Apply page row patches and cursor metadata together in TypeScript, Python, Dart, Rust and Go clients. Inserting a row before another row now preserves both. Generate valid Dart types for nullable model fields such as page cursors.
