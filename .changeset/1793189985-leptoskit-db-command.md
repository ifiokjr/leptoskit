---
"release": minor
---

# Generate migrations with `leptoskit db`

A `[db]` table in `leptoskit.toml` turns on `leptoskit db`, which writes migrations from the app's `#[db::table]` structs instead of by hand. `leptoskit db migration create add_posts` reads the tables from the db crate (which calls `leptoskit::db::export_schema!()` once), compares them with the snapshot beside the latest migration, and writes the SQL, a new snapshot, and the registry entry. It asks whether a column that disappeared was renamed, refuses to drop data without `--allow-destructive`, and writes empty data migrations with `--empty`. `leptoskit db check` fails CI when the tables changed without a migration; `migrate`, `status`, and `reset` work against the database in `DATABASE_URL`. The `leptoskit` crate gains `db`, `db-server`, `db-postgres`, and `db-sqlite` features that expose `leptoskit::db`.
