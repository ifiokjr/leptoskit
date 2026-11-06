---
leptoskit_db: major
---

# leptoskit_db: tables as Rust structs

`leptoskit_db` is leptoskit's database layer: tables declared as Rust structs, typed IDs, client-safe public projections that leave out private and secret columns, typed queries and writes, links, access policies, and migrations.

Enable `db` on `leptoskit` for the client side, which has no database code, and `db-postgres` or `db-sqlite` on the server; `db-geo` and `db-vector` add geospatial and vector columns. `leptoskit db` diffs the tables and writes migrations, and `leptoskit db studio` browses the data in development. Auth and IDP build their schema packs on it.
