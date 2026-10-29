---
"release": minor
---

# Apply database migrations at startup

`leptoskit::db::migrations!("../migrations")` embeds every migration listed in `migrations/registry.txt`, and `db.migrate(MIGRATIONS)` applies the pending ones, each in its own transaction, recording a checksum of its SQL. An applied migration that was edited afterwards, or one the build doesn't know, stops the server instead of being skipped. Servers starting together on Postgres take turns through an advisory lock, and SQLite migrations that rebuild tables run with foreign keys off and verified before they commit. `db::testing::sqlite()` gives tests an in-memory database with every table created from the code.
