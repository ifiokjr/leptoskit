---
leptoskit_db_macros: major
---

# leptoskit_db_macros: the database macros

`leptoskit_db_macros` provides the macros behind leptoskit's database layer: `#[db::table]`, `#[db::link]`, `#[db::view]`, `#[db::policy]`, `#[db::mixin]`, `#[db::polymorphic]`, `db::migrations!`, and the `Shape`, `Enum`, and `Scalar` derives.

Apps use them through `leptoskit::db` or `leptoskit_db`; this crate is the macro implementation. Every mistake it can catch is a compile error that says what to write instead.
