---
leptoskit_db_macros: breaking
---

# leptoskit_db_macros: the table and enum macros

`leptoskit_db_macros` provides `#[db::table]`, `#[derive(db::Enum)]`, and `#[derive(db::Scalar)]` behind leptoskit's database layer.

Apps use them through `leptoskit::db` or `leptoskit_db`; this crate is the macro implementation. Every mistake it can catch is a compile error that says what to write instead.
