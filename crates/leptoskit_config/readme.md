<h1 align="center">leptoskit_config</h1>

<p align="center">
  <a href="https://ifiokjr.github.io/leptoskit/"><img src="https://raw.githubusercontent.com/ifiokjr/leptoskit/main/docs/src/assets/mark.svg" alt="leptoskit" width="64"></a>
</p>

<p align="center">
  <strong>leptoskit_config</strong> loads and validates `leptoskit.toml`: app identity, platforms, servers, native targets, code push channels, plugin declarations, and lint settings.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://docs.rs/leptoskit_config"><strong>Docs</strong></a> ·
  <a href="https://github.com/ifiokjr/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<p align="center">
  <a href="https://crates.io/crates/leptoskit_config"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_config.svg"></a>
  <a href="https://docs.rs/leptoskit_config"><img alt="docs.rs" src="https://docs.rs/leptoskit_config/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<br>

## Overview

One `leptoskit.toml` at the project root describes the app: identity and version, default platform, server address and port, build features, native targets and viewport, code push channels, and plugin declarations. Everything is typed, defaulted, and round-trippable — the CLI scaffolds it, edits it, and reads it through this crate.

Unknown fields are rejected, so typos fail at load time instead of silently doing nothing. The version 0 schema is unstable; breaking changes are expected until the first version 1 schema.

The [schema and migration guide](https://ifiokjr.github.io/leptoskit/reference/schemas.html) lists public schema URLs and the major-version policy. Run `leptoskit migrate --dry-run` to preview upgrades of an existing file, then `leptoskit migrate` to apply them. Unversioned files remain readable; migration stamps the contract and preserves comments. Enable this crate's `schema` feature to generate JSON Schema from the same Rust types.

## How it fits

The CLI, build pipeline, dev server, and native hosts all parse the same configuration, so they agree on what the app is. Unknown keys and invalid combinations fail with the file and field before a build starts.

## Use it

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

## Learn more

- [Crate documentation](https://docs.rs/leptoskit_config)
- [leptoskit documentation](https://ifiokjr.github.io/leptoskit/)
- [Repository](https://github.com/ifiokjr/leptoskit)
- [Configuration](https://ifiokjr.github.io/leptoskit/guide/03-configuration.html)
- [Schema](https://ifiokjr.github.io/leptoskit/reference/schemas.html)
