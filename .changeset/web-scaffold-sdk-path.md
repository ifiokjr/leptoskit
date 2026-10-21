---
"release": patch
---

# Scaffold web projects against a local SDK

`leptoskit new --sdk-path <repo>` now works for web projects as well as native ones: the client depends on the repository's `crates/leptoskit` by path, so a scaffold can use features that are not released yet. Previously the flag was rejected for web projects, and a fresh web scaffold could only build against the published release.
