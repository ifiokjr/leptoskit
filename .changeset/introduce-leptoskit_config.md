---
leptoskit_config: breaking
---

# leptoskit_config: project configuration

`leptoskit_config` loads and validates `leptoskit.toml`: app identity, platforms, servers, native targets, code push channels, plugin declarations, and lint settings.

The CLI, build pipeline, dev server, and native hosts all parse the same configuration, so they agree on what the app is. Unknown keys and invalid combinations fail with the file and field before a build starts.
