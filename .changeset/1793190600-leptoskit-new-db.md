---
"release": minor
---

# Scaffold a database with `leptoskit new --db`

`leptoskit new my_app --db postgres` (or `--db sqlite`) creates a project that already uses the database: a `my_app_db` crate declaring an example `Note` table, an empty `migrations/` directory, a `[db]` table in `leptoskit.toml`, a server that applies pending migrations before serving, and a backend with `GET /api/notes` and `POST /api/notes`. Run `leptoskit db migration create init` and `leptoskit dev` to see it work. A new guide page, "The database", covers declaring tables, private and secret columns, queries, and migrations.
