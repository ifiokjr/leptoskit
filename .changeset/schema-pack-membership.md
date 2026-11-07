---
"release": minor
---

# Schema packs declare their membership

`leptoskit::db::packs!` now records the schema packs an app expects, and export records each pack's identity and the tables it owns in `schema.json` (format 2). A pack that is declared but exports no tables is refused before the first migration, and a pack the latest snapshot or a development push recorded can no longer disappear as an ordinary destructive migration: `db migration create`, `db push`, `db migration consolidate`, and the development watch require `--drop-pack <name>` for each pack whose removal is intended. `--allow-destructive` does not cover a pack removal, so an omitted dependency or feature cannot silently drop a library's tables. Changes inside a pack that is still present stay ordinary migrations.

Pack crates declare their identity and tables once with `leptoskit_db::schema_pack!`, which keeps the pack's tables in the export even when the app only names the pack in `packs!`. `leptoskit_auth`, `leptoskit_idp`, and `leptoskit_rpc` already do. Existing format 1 snapshots remain readable; they carry no membership evidence, so a pack that vanished before the snapshot was written cannot be detected from it, and the next migration writes format 2.
