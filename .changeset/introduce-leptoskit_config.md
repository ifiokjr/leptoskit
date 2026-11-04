---
leptoskit_config: major
---

# leptoskit_config: project configuration

`leptoskit_config` loads and validates `leptoskit.toml`: app identity and platforms, server and build settings, native targets and plugin grants, code push channels, routes, translations, the database, sign-in, infrastructure, browser extensions, store publishing, plugins, formatting, and lint settings.

The CLI, build pipeline, dev server, and native hosts all read the same typed configuration, so they agree on what the app is. Unknown keys fail at load time instead of being ignored. The schema is versioned (version 0, unstable until version 1); `leptoskit migrate` upgrades an older file, and the `schema` feature generates its JSON Schema.
