---
"release": minor
---

# Add `leptoskit_db_schema` for generated migrations

The first piece of leptoskit's built-in database layer. A database schema (tables, typed columns, keys, unique and partial indexes, foreign keys, checks, comments, and namespaces) is described once and compared with the snapshot committed beside the previous migration, producing a step-by-step plan and the SQL that applies it, for Postgres and SQLite. Renames come from explicit hints, or from a question when a column or table appears while a similar one disappears, so data is never dropped by a guess. Every step says what it risks for existing rows, and steps that delete data are called out. SQLite migrations rebuild tables whenever `ALTER TABLE` can't make a change, keeping every row and checking every foreign key. Apps use it through the upcoming `leptoskit db` commands rather than directly.
