---
"release": minor
---

# Read-only connections, in-memory conditions, read sets

`leptoskit_db` gains what live queries build on:

- **`db::Read`.** Every read (queries, `get`, `find`, link accessors, shapes, views) takes `&impl db::Read`, and writes still take `&impl db::Connection`, which now extends it. `Db` and `Tx` are both, so existing code keeps compiling; code that imported `Connection` only to call `dialect()` can drop the import, since `Db::dialect` and `Tx::dialect` are now inherent.
- **`Expr::eval`.** A condition can be checked against one row's values without the database, in SQL's three-valued logic. It answers "false" only when both Postgres and SQLite would certainly not match, and "unknown" for anything dialect- or collation-sensitive, or that reads other rows.
- **`db::Tracked`.** A read-only connection that records each read: the table, the filter combined with the access policy and soft deletes, the order and limit, and the keys of the rows returned. Policy subqueries, link tables, and each level of a shape get their own entries.
- **Raw SQL and views declare what they read.** `Statement::new(sql, args).reads::<Post>().fetch(&db)` and `#[db::view(reads(Post))]`; a tracked read that doesn't say fails with `DbError::UndeclaredRead`.
- **`HasId` on client types.** `Public…` types, projections, and public shapes that select their table's `id` implement `HasId<Table>`. `HasId` now takes the table as a parameter (`HasId<T = Self>`) and no longer requires `Table`.
