---
"release": minor
---

# The live hub re-runs only what a write could change

`leptoskit_db` gains `ReadEntry::could_change(change, dialect)`: it says `false` only when the written row belonged to another table, or neither its before nor after image met the read's condition, so a row outside a query never re-runs it. Conditions the matcher can't decide — dialect differences, subqueries, settings, a missing column — count as a change.

`rpc::server::Live` now reads the change log rather than a per-connection invalidation feed, keeps each subscription's read set, and matches every commit against it. A write to a table a query never read no longer re-runs it; a write whose row could match does. Transitions carry the commit's version, `Everything` re-runs on a pruned log, and the origin and identity re-checks are unchanged. `LiveStats::reruns` counts only the queries actually run again.
