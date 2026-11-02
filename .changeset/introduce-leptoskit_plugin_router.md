---
leptoskit_plugin_router: breaking
---

# leptoskit_plugin_router: file routes as a build plugin

`leptoskit_plugin_router` is the file router as a leptoskit plugin: it generates the typed `Route` module before cargo runs and answers `plugin:router routes`.

Generated apps enable it by default, so `src/routes/` is always in step with the build. The runtime and derive live in `leptoskit_router`; this crate only owns the build hook.
