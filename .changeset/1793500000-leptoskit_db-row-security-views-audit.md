---
"release": minor
---

# Row-level security covers views and the audit log

With `[db] policies = "rls"`, plain views on Postgres now read their tables as whoever reads them (`security_invoker`), so the tables' policies filter a view too, whoever owns it. Before, a view owned by a superuser, such as one created by a migration run as one, read every row. Materialized views are refreshed bypassing the policies, so they hold every row whoever refreshes them.

The audit log gets a policy of its own: raw SQL reads it only from a request holding an `AuditReader`, requests can add entries, and only `db.system(…)` can change or remove one. Before, the audit log had no policy, so raw SQL from any request could read it.
