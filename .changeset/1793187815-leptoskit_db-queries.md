---
"release": minor
---

# Query and write tables on Postgres and SQLite

`leptoskit_db` connects with `Db::connect` to Postgres (`tokio-postgres`, TLS by default) or SQLite (`rusqlite`, bundled, the same SQLite as the on-device plugin), and every `#[db::table]` gets typed queries and writes: `User::query().filter(|u| u.karma.gt(10) & u.name.is_not_null()).order_by(|u| u.karma.desc()).limit(20).all(&db)`, `count`, `exists`, `find`, `get`, `NewUser { … }.insert(&db)`, `upsert` on any unique index, `save`, `delete`, bulk `update()`, and `db.transaction(async |tx| …)`, which commits on `Ok` and rolls back on `Err` or cancellation. Writes check the columns' rules first, timestamps fill themselves in, soft-deleted rows stay hidden until a query asks for them, and errors tell a taken unique value, a broken foreign key, and a failed check apart without putting SQL or values in their message.
