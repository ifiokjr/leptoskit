# Configuration schema and migrations

The JSON Schema for `leptoskit.toml` is generated from the Rust configuration types, including field documentation, defaults, enum choices, and unknown-field checks. It uses JSON Schema draft 2020-12. The documentation build copies the JSON files unchanged and publishes them with the book.

| Contract             | Public URL                                                                                       |
| -------------------- | ------------------------------------------------------------------------------------------------ |
| Latest alias         | [schema.json](https://ifiokjr.github.io/leptoskit/schema.json)                                   |
| Latest named alias   | [leptoskit.schema.json](https://ifiokjr.github.io/leptoskit/schemas/leptoskit.schema.json)       |
| Version 0 (unstable) | [leptoskit.v0.schema.json](https://ifiokjr.github.io/leptoskit/schemas/leptoskit.v0.schema.json) |

For autocomplete and diagnostics in schema-aware TOML editors such as Taplo, put this directive at the top of the file. Generated projects already include it:

```toml
# :schema https://ifiokjr.github.io/leptoskit/schemas/leptoskit.v0.schema.json

schema_version = 0

[app]
name = "my_app"
```

The current contract is **version 0, which is unstable**. Breaking changes are expected until the first version 1 schema. The v0 URL can change incompatibly during this period, even when pinned. Version 1 will establish the first stable contract; from then on, incompatible changes require a new major and migration step, while compatible fields can be added within a major.

The aliases follow the latest contract. Configuration versions are non-negative integers, independent of LeptosKit's crate version. The CLI also validates paths and cross-field constraints when loading configuration.

## Upgrade an existing file

Unversioned files remain readable with today's configuration shape. The migration command distinguishes these legacy files from explicit version 0 files:

```sh
leptoskit migrate --dry-run # print the complete proposed TOML without writing
leptoskit migrate --check   # exit nonzero if the file needs migration
leptoskit migrate           # validate and replace the file atomically
```

The command searches the current directory and then its parents. Use `--file path/to/leptoskit.toml` to target a specific file. Migration preserves unrelated settings, comments, and quoting, sets `schema_version`, and updates the schema directive. Repeating it leaves the file unchanged.

The legacy → version 0 migration moves the old `build.wasm_opt` setting to `bundle.wasm_opt` and stamps `schema_version = 0`. If both locations are present, migration fails so you can choose which value to keep. Invalid TOML, unknown fields, invalid settings, and future schema versions also fail before writing. Normal builds only read configuration; they never rewrite it. Future versions, including version 1, require a newer CLI.
