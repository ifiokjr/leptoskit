---
"release": minor
---

# Live tables and change capture

`leptoskit_db` can now record every write to a table for live queries, in the writing transaction, whoever made the write.

- **`#[db::table(live)]`** marks a table live. The macro implements `db::LiveTable` and records the columns to capture: every column except `secret` ones and fields marked `#[db(live = skip)]`.
- **The `live` pack.** Migrations create `live.commits` (one row per committed transaction that wrote a live table), `live.changes` (one row per written row), and on Postgres `live.pending`, the sequences, and the `live.capture()` and `live.seal()` functions. Each live table gets capture triggers. Postgres allocates the version at commit under a transaction-scoped advisory lock and sends `NOTIFY leptoskit_live`; SQLite stamps the version on the transaction's changes just before it commits.
- **`db.changes().since(version).all()`** reads the committed changes in version order. `db.prune_live(retention)` deletes changes and commits older than the resume window. The live hub in `leptoskit_rpc` is the first consumer.
- **Schema objects.** `leptoskit_db_schema` now models sequences, functions (body plus checksum), and triggers, including deferred constraint triggers. The diff creates, replaces, and drops them; a SQLite table rebuild recreates its triggers.
- **Actor.** A Postgres transaction sets `leptoskit.user_id` from the request's actor, and SQLite records it on the commit row. `live.seal()` uses it for `live.commits.actor`.
