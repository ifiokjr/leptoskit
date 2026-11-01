---
leptoskit_plugin: breaking
---

# leptoskit_plugin: build-time plugins

`leptoskit_plugin` is the build-hook system shared by leptoskit plugins and leptos plugins: feature flags, environment variables, asset directories, and post-build reports.

A plugin crate implements `LeptoskitPlugin` and is declared in `leptoskit.toml`; leptos plugins are forwarded to the app build as cargo features. Runtime native plugins use the bridge instead; this is the build-time half.
