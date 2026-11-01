---
"release": minor
---

# Read-only connections, read sets, in-memory conditions

`leptoskit_db` and `leptoskit_rpc` gain what live queries build on:

- **`db::Read`.** Every read (queries, counts, `get`, `find`, link accessors, shapes, searches, `nearest`, polymorphic reads, and views) takes `&impl db::Read`, and writes still take `&impl db::Connection`, which now extends it. `Db` and `Tx` are both, so code passing them keeps compiling. `Db::dialect` and `Tx::dialect` are now inherent, so code that imported `Connection` only to call `dialect()` can drop the import; code that called the hidden `fetch` on a `Db` imports `db::Read` instead.
- **`db::Tracked`.** A read-only connection that records each read: the table, the filter combined with the access policy and soft deletes, the order, limit and offset, the kind of read, and the keys of the rows returned. Policy subqueries, link tables, each level of a shape, and searches get their own entries.
- **Raw SQL and views declare what they read.** `Statement::new(sql, args).reads::<Post>().fetch(&db)` and `#[db::view(reads(Post))]`; a tracked read that doesn't say fails with `DbError::UndeclaredRead`.
- **`Expr::eval`.** A condition can be checked against one row's values without the database, in SQL's three-valued logic. It answers "false" only when both Postgres and SQLite would certainly not match, and "unknown" for anything dialect- or collation-sensitive, or that reads other rows.
- **`HasId` on client types.** `Public…` types, projections, and public shapes that select their table's `id` implement `HasId<Table>`. `HasId` now takes the table as a parameter (`HasId<T = Self>`) and no longer requires `Table`.
- **Queries can't write.** `QueryCx::db()` is a `Tracked<Db>`, so a write in an `#[rpc::query]` body fails to compile. `QueryCx::reads()` returns its read set, and `rpc::server::ReadSink`, passed in `Invocation::reads`, collects it from a query run by name.
