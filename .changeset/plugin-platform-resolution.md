---
"release": minor
---

# Build with the same plugins on every platform

A plugin listed in `native.plugins` that has no implementation for the platform being built, or that the platform's app can't run yet, no longer stops the build. The build leaves it out with one warning, such as `warning: haptics has no Linux implementation; calls fail with Unavailable`, and calls to it on that platform fail with `Unavailable`. Set `native.missing-plugins = "error"` to keep failing instead. Windows, Linux, and terminal apps now build with plugins listed; they leave every plugin out until their runners can register them.

HarmonyOS apps leave out worker plugins, async plugins, and plugin services, which their host can't run. Async plugins such as `http`, `shell`, and `websocket` previously built and then tore the page down on their first call.

Plugin crates can register per platform with `[package.metadata.leptoskit.runtime.<platform>]` tables (`ios`, `macos`, `android`, `harmonyos`, `windows`, `linux`, `tui`) holding `rust-type` or `rust-factory`, `rust-worker`, `rust-async`, or `unavailable = true`, which opts a platform out without a warning. A platform's table wins over the shared Rust fields, which win over its native type. `apple-module` and `apple-type` must now be declared together.
