---
leptoskit_plugin: major
---

# leptoskit_plugin: build-time plugins

`leptoskit_plugin` is the build-hook system shared by leptoskit plugins and leptos plugins: generated code, feature flags, environment variables, asset directories, post-build reports, and a plugin's own `plugin:<name>` commands.

Plugins are declared in `leptoskit.toml`. A Rust plugin crate implements `LeptoskitPlugin`; a separate program in any language answers each hook over a versioned JSON protocol; and leptos plugins are forwarded to the app build as cargo features. Runtime native plugins use the bridge instead; this is the build-time half.
