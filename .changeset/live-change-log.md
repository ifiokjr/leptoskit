---
"release": minor
---

# Capture live changes with triggers and commit versions

Live tables now record their writes in the database itself, so every write reaches subscribers, whoever makes it, and nothing is lost when a connection drops.

- **Every writer counts.** Marking a table `live` gives it capture triggers in its next migration, which record each insert, update, and delete in the `live` pack's change log (`live.changes` and `live.commits`) in the writing transaction. Raw SQL, `psql`, other services, and the database's own `on_delete` cascades are captured like the app's own writes.
- **One version line.** Each commit that wrote a live table takes the next version: on Postgres at commit, under a transaction-scoped advisory lock, so versions become visible in order; on SQLite just before the pool commits. Rolled-back transactions take none.
- **Feeds read the log.** `db.changes()` works as before, and a feed now catches up from the log after a lagging subscriber or a lost `LISTEN` connection instead of returning `Invalidation::Everything`. `Feed::next_versioned()` returns each invalidation's version, and `db.changes_since(version)` resumes after it. `Everything` remains for a log pruned past the feed, or a table this build doesn't know.
- **Smaller columns, shorter logs.** `#[db(live = skip)]` keeps a column out of the log, as `secret` columns always are, and `Table::LIVE_COLUMNS` lists what is copied. `db.prune_changes(retention)` deletes old entries.
- **For servers.** `db.change_log(version)`, on a `system` handle, reads every commit with its rows' keys and images, for a hub that decides itself who may see what. `Tx::set_origin` records what made a commit, and RPC mutations record their own names.
- **Lighter writes.** Live tables no longer take the audit log's write path, and a transaction no longer buffers changes to publish. Audited tables behave as before.
- **`leptoskit db check --live`** checks a database's capture triggers, functions, and sequences against the schema. `leptoskit_db_schema` models sequences, functions, and triggers, including deferred constraint triggers, and diffs and renders them on both databases.
