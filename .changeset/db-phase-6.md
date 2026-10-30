---
"release": minor
---

# Live changes, push mode, and branch databases

`#[db::table(live)]` publishes every committed write: `db.changes().await?` gives a request a feed of `Invalidation`s naming the tables and keys that changed, filtered by its `select` policy before and after each change, so nobody learns of rows they can't see. Postgres carries changes between servers with `LISTEN`/`NOTIFY`; SQLite keeps them in the process. With `leptoskit_query`'s `db` feature, `table_tag`, `row_tag`, and `QueryClient::apply_invalidation` refetch exactly the queries a change touches.

`[db] watch = "push"` makes `leptoskit dev` apply table changes to the development database without writing migrations; `leptoskit db migration create` then writes one migration for all of them and the database takes it over, rows intact. Pushes that delete data need `leptoskit db push --allow-destructive`, and a database with pushes refuses migrations it hasn't had.

On the development Postgres each git branch gets its own database, copied from the main branch's the first time; `leptoskit dev` follows `git switch`, and `leptoskit db branch list` and `prune --yes` clean up after finished branches. `branch_databases = false` turns it off.

Also: `Links::set` replaces a whole link set at once, shapes take `with_deleted` on nested rows, the audit log records rows the database deletes or empties through `on_delete` cascades, materialized views take `index(…)` and `unique(…)` and `refresh_concurrently()`, views can read other views, and shapes can start from a view.
