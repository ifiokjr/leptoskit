---
leptoskit_db: breaking
---

# leptoskit_db: tables as Rust structs

`leptoskit_db` is leptoskit's database layer: tables declared as Rust structs, typed IDs, client-safe public projections, queries, and migrations.

It gives shared code a typed database without hand-written SQL; `leptoskit db` diffs the schema and writes migrations. Auth and IDP build their schema packs on it.
