---
"release": minor
---

# Capture live changes by logical replication

An app that commits thousands of transactions a second to live tables can now record their changes from Postgres's write-ahead log instead of with triggers. Set `[db] live_capture = "replication"`: the next migration drops the capture triggers and publishes the live tables, and `Db::capture_changes()` reads each committed transaction from a logical replication slot (`pgoutput`) into the same change log, with versions in commit order and each commit's write-ahead log position. Writers pay nothing extra, and feeds, invalidations, `Db::change_log`, and clients are unchanged. One server captures at a time, chosen by an advisory lock, and another takes over when it stops; the slot moves only after the log has the rows. `Db::live_stats()` reports how far capture lags (`live_capture_lag`), `Db::live_caught_up()` waits until the log has every commit made so far, and `leptoskit db check --live` warns when nothing is capturing. It needs `wal_level = logical` and a role with `REPLICATION`.

On Postgres the change log is now partitioned, commits by version and changes by their order, and the next migration rebuilds an existing log into partitions. Pruning keeps its marker commit and drops whole partitions rather than deleting rows. `Db::live_maintenance(retention, interval)` makes partitions ahead and prunes on a schedule, taking turns across servers, and `rpc::server::Jobs` starts it, and replication capture, with the workers: `[db] live_retention_minutes` (15 by default) says how much of the log to keep, and `Jobs::from_configs` reads it.
