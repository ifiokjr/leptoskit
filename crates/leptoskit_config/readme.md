<p align="center">
  <a href="https://ifiokjr.github.io/leptoskit/"><img src="https://raw.githubusercontent.com/ifiokjr/leptoskit/main/docs/src/assets/mark.svg" alt="leptoskit" width="64"></a>
</p>

# leptoskit_config

> Configuration loading for leptoskit projects.

One `leptoskit.toml` at the project root describes the app: identity and version, default platform, server address and port, build features, native targets and viewport, code push channels, and plugin declarations. Everything is typed, defaulted, and round-trippable — the CLI scaffolds it, edits it, and reads it through this crate.

```toml
[app]
name = "counter"
version = "0.1.0"
default_platform = "native"

[codepush]
enabled = true
server = "https://cp.example.com"
channel = "production"

[[plugins]]
name = "leptos_meta"
leptos = true
```

Unknown fields are rejected, so typos fail at load time instead of silently doing nothing.
