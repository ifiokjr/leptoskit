---
"release": patch
---

# `--deny-warnings` catches cargo's warnings too

`leptoskit lint --deny-warnings` now fails on cargo's own warnings, such as an unused dependency, as well as rustc's and clippy's. It denies them through cargo's `build.warnings` setting, so switching it on no longer rebuilds the project.

`leptoskit_plugin_secure_store`'s `async-storage` feature no longer compiles its storage dependencies into iOS and Android builds, which have no adapter to use them.
