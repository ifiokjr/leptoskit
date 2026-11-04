---
leptoskit_plugin_sqlite: major
---

# leptoskit_plugin_sqlite: private SQLite

`leptoskit_plugin_sqlite` is the leptoskit plugin for bounded, application-private SQLite databases with typed bindings and the same bundled engine on every native platform.

Add `leptoskit_plugin_sqlite` as a direct dependency and select the `sqlite` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::Sqlite` for parameterized writes, read-only queries, and atomic transaction batches. The database runs in Rust on the host's worker, so iOS, Android, and macOS behave the same; Windows and Linux hosts that register plugins themselves get the same engine, and the browser has no SQLite backend. Guests cannot choose file paths, attach databases, or run pragmas, and each database is capped at 64 MiB by default, with limits on statement, result, and transaction sizes.
