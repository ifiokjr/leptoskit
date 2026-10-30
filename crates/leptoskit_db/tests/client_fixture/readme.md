# leptoskit_db_client_fixture

A test fixture, never published: the client half of `#[db::table]` compiled for the browser.

A table declared with `#[table]` has to be usable from client code, for validation and forms, without dragging in the database. The `server` feature of `leptoskit_db` is what brings in the SQL drivers and schema generation; without it, only the types remain. This crate declares a table (`User`, with public, private, secret, flattened, and optional fields) and some scalars, and depends on `leptoskit_db` without the `server` feature, so a client build proves the macro expands to something that compiles with no server code.

CI checks two things in the `use coverage and portability` job of `.github/workflows/ci.yml`. It builds the crate for `wasm32-unknown-unknown`, and it runs `cargo tree` on it to fail if `sqlx`, `inventory`, `leptoskit_db_schema`, or `regex` reached the dependency tree:

```sh
cargo check -p leptoskit_db_client_fixture --target wasm32-unknown-unknown
```
