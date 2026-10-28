---
"release": minor
---

# The file router is a plugin with its own commands

Routing is now declared like every other plugin — `[[plugins]] name = "router"` in `leptoskit.toml` — with `dir` and the new `output` options placing the generated module. The router generates through the same plugin lifecycle as everything else, so `leptoskit build`, `dev`, and `native` regenerate the module through the plugin registry, and the dev watcher excludes whatever any plugin generates.

Its commands live under the new plugin namespace: `leptoskit plugin:router routes` (and `--check`) replace the top-level `leptoskit routes`, which still forwards with a deprecation note. `leptoskit --help` lists the plugin commands a project activates. `leptoskit migrate` rewrites a legacy `[router]` table into the plugin form; the table keeps working until then.
