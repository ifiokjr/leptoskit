<p align="center">
  <a href="https://ifiokjr.github.io/leptoskit/"><img src="https://raw.githubusercontent.com/ifiokjr/leptoskit/main/docs/src/assets/mark.svg" alt="leptoskit" width="64"></a>
</p>

# leptoskit_config

> Configuration loading for leptoskit projects.

One `leptoskit.toml` at the project root describes the app: identity and version, default platform, server address and port, build features, native targets and viewport, code push channels, and plugin declarations. Everything is typed, defaulted, and round-trippable — the CLI scaffolds it, edits it, and reads it through this crate.

```toml
#:schema https://ifiokjr.github.io/leptoskit/schemas/leptoskit.v0.schema.json

schema_version = 0

[app]
name = "counter"
version = "0.1.0"
default_platform = "native"

[codepush]
enabled = true
server = "https://cp.example.com"
channel = "production"
public_keys = [
	"a0d1b6b0b6a5f6cbe4f0e2f6dbb2a1e7b0c3d8f9e1a2b3c4d5e6f708192a3b4c",
]

[[plugins]]
name = "leptos_meta"
leptos = true
```

Unknown fields are rejected, so typos fail at load time instead of silently doing nothing. The version 0 schema is unstable; breaking changes are expected until the first version 1 schema.

The [schema and migration guide](https://ifiokjr.github.io/leptoskit/reference/schemas.html) lists public schema URLs and the major-version policy. Run `leptoskit migrate --dry-run` to preview upgrades of an existing file, then `leptoskit migrate` to apply them. Unversioned files remain readable; migration stamps the contract and preserves comments. Enable this crate's `schema` feature to generate JSON Schema from the same Rust types.
