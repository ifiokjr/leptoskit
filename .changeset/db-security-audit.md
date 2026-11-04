---
"release": patch
---

# Close an upsert race and keep large writes working

- An upsert under row policies now locks the row it would overwrite before checking the update policy, and inserts plainly when there is none. Before, on Postgres, an upsert racing another transaction's insert of the same unique value could overwrite that row, `author_id` included, when only the insert policy had been checked.
- A commit of more than 500 rows to a SQLite table with a read policy no longer breaks every live feed that checks that policy. SQLite allows at most 500 terms in one compound `SELECT`.
- Bulk updates, soft-delete cascades, and supertype removals check and change their rows in batches of 500. They no longer fail past the databases' bind limits, or overflow the stack on tables keyed by two columns.
- The studio no longer filters or sorts by `secret` columns, which could read a secret back one character at a time.
