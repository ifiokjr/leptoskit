# leptoskit_db_pack_fixture

A test fixture, never published: a schema pack, the way a library ships one.

A library can declare tables with `#[table(pack = "…")]` so they join an app's schema under the library's own namespace. Apps read those tables, but only the library's API writes them. This crate is such a library. It declares two tables in the `sample` pack, `Note` and `Signature` (a note's signatures, linked by a cascading foreign key), and one write path, `add_note`.

Its tests play the app:

- `tests/pack_only.rs` is the isolated consumer: its only reference to the pack is `leptoskit_db::packs!`, and it checks that both dialect exports still contain every table and the pack's membership.
- `tests/pack.rs` links the pack with `leptoskit_db::packs!`, checks that both tables land in the schema as `sample.notes` and `sample.signatures` on Postgres (and export for SQLite too), and migrates an in-memory SQLite database to write through `add_note` and read the result back.
- `tests/ui/` holds the compile-fail case: an app that tries to insert, save, or update a pack's table directly does not build.

The crate declares its identity with `leptoskit_db::schema_pack!`, which `packs!` uses to link every table even when the app never mentions one of the pack's types.

```sh
cargo test -p leptoskit_db_pack_fixture
```
