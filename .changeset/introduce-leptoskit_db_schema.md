---
leptoskit_db_schema: breaking
---

# leptoskit_db_schema: schema diff and migrations

`leptoskit_db_schema` holds the schema model, snapshot format, diff, and SQL renderers behind leptoskit's generated database migrations.

`leptoskit db` and the `leptoskit_db` runtime use it; apps never depend on it. It is pure - no connection and no files - and renders Postgres and SQLite SQL from the same plan, with each step's risk assessed against existing rows.
