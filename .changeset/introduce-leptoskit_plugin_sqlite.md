---
leptoskit_plugin_sqlite: breaking
---

# leptoskit_plugin_sqlite: private SQLite

`leptoskit_plugin_sqlite` is the leptoskit plugin for bounded, application-private SQLite databases with typed bindings and the same bundled engine on every native platform.

Add `leptoskit_plugin_sqlite` as a direct dependency and select the `sqlite` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::Sqlite` rather than resolving the plugin on every call. The database lives in Rust, not a platform channel or JavaScript engine, so iOS, Android, macOS, Windows, and Linux behave the same; the browser has no SQLite plugin.
