---
"leptoskit_plugin": minor
---

# Plugin hooks for codegen, commands, native needs, serving

`LeptoskitPlugin` grows defaulted hooks, so existing plugins compile unchanged: `generate` runs before cargo on every `leptoskit build` and dev-server rebuild and records the files it wrote so the dev watcher ignores them, `run_command` answers a plugin's own `plugin:<namespace> <command>` commands, `native_integration` declares the Android permissions, Maven dependencies, and Apple usage descriptions a generated host needs, and `metadata` declares the plugin's CLI namespace and supported platforms — a build for an unsupported platform now fails naming the plugin.

`before_serve`, which existed but was never called, now runs before the `dev` and `serve` servers start.
