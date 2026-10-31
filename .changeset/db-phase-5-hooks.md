---
"release": minor
---

# Hooks, an audit log, and views in the database

`hooks(before_write, after_insert, after_update, after_delete)` on `#[db::table]` runs your own `BeforeWrite`, `AfterInsert`, `AfterUpdate`, and `AfterDelete` code inside every single-row write's transaction, so a failing hook rolls the write back. `before_write` sees whether it's an insert and the row as stored before an update. Bulk updates and deletes, which never load rows, fail with `DbError::HooksSkipped` on a table whose hooks they'd skip, unless told `.allow_skipping_hooks()`.

`#[db(created_by)]` and `#[db(updated_by)]` record who inserted and last wrote a row, from the request's `Actor`, which sign-in adds for every signed-in request. A required one refuses a write with nobody acting. `#[db(slug(from = title), unique)]` fills in a unique URL slug on insert, such as `hello-world-2`.

`#[db::table(audit)]` records every insert, update, and delete in `_leptoskit_audit`, in the same transaction: who made it, the reason a `db.system("…")` bypass gave, and the row before and after, never its `secret` columns. A request reads no entry unless its context holds an `AuditReader`.

`#[db::view(name = "author_stats", sql = "SELECT …")]` declares a view that the migrations create and that's read like a table under its own `select` policies; `materialized` keeps its rows until `AuthorStats::refresh(&db)`, on Postgres and SQLite alike.
