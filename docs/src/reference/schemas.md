# Configuration schema and migrations

The JSON Schema for `leptoskit.toml` is generated from the Rust configuration types, including field documentation, defaults, enum choices, and unknown-field checks. It uses JSON Schema draft 2020-12. The documentation build copies the JSON files unchanged and publishes them with the book.

| Contract           | Public URL                                                                                       |
| ------------------ | ------------------------------------------------------------------------------------------------ |
| Latest alias       | [schema.json](https://ifiokjr.github.io/leptoskit/schema.json)                                   |
| Latest named alias | [leptoskit.schema.json](https://ifiokjr.github.io/leptoskit/schemas/leptoskit.schema.json)       |
| Version 1          | [leptoskit.v1.schema.json](https://ifiokjr.github.io/leptoskit/schemas/leptoskit.v1.schema.json) |

For autocomplete and diagnostics in schema-aware TOML editors such as Taplo, put this directive at the top of the file. Generated projects already include it:

```toml
# :schema https://ifiokjr.github.io/leptoskit/schemas/leptoskit.v1.schema.json

schema_version = 1

[app]
name = "my_app"
```

The aliases follow the latest contract. Pinning the major URL keeps an older project on its own contract. Configuration versions are positive integers, independent of LeptosKit's crate version; compatible fields can be added within a major, while incompatible changes require a new major and migration step. The CLI also validates paths and cross-field constraints when loading configuration.

## Upgrade an existing file

Unversioned files remain readable with today's configuration shape. They are treated as legacy version 0 by the migration command:

```sh
leptoskit migrate --dry-run # print the complete proposed TOML without writing
leptoskit migrate --check   # exit nonzero if the file needs migration
leptoskit migrate           # validate and replace the file atomically
```

The command searches the current directory and then its parents. Use `--file path/to/leptoskit.toml` to target a specific file. Migration preserves unrelated settings, comments, and quoting, sets `schema_version`, and updates the schema directive. Repeating it leaves the file unchanged.

Version 0 → 1 moves the old `build.wasm_opt` setting to `bundle.wasm_opt`. If both locations are present, migration fails so you can choose which value to keep. Invalid TOML, unknown fields, invalid settings, and future schema versions also fail before writing. Normal builds only read configuration; they never rewrite it. Explicit older versions require `leptoskit migrate`, and future versions require a newer CLI.

## Maintain the contract

Generate and check the assets from the repository root:

```sh
devenv shell schema:update
devenv shell schema:check
devenv shell docs:build
```

Do not edit generated JSON by hand. `schema:check` runs in local lint and pre-push checks and in CI. Tests validate real project configurations against the schema and exercise historical migrations. The docs workflow verifies that the built site contains the exact committed schema bytes.

For a breaking configuration change:

1. Increment `SCHEMA_VERSION` in `leptoskit_config`. The version constraint and pinned editor URL derive from it.
2. Add the next explicit step to `migration::migrate`. Keep every older step, and reject missing steps and future versions.
3. Add a historical fixture and verify that migration preserves its meaning and comments, rejects conflicts, and is idempotent.
4. Run `schema:update`. It updates both latest aliases and writes the current major's versioned asset. It never removes or rewrites older majors.
5. Preserve published older contracts and add the new major URL to this page. Update schema tests to include the new asset.
