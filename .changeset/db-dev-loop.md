---
"release": minor
---

# Run a dev Postgres and count linked rows in policies

A Postgres app no longer needs a database set up by hand. When `DATABASE_URL` isn't set, `leptoskit dev` uses devenv's `services.postgres` if `devenv.nix` turns it on, and otherwise runs an embedded Postgres: the first run downloads it once into `~/.cache/leptoskit/postgresql`, and the data stays in the project's `.leptoskit/db/postgres`. The app server gets its URL, `leptoskit db migrate`, `status`, and `reset` use the same database, and `[db] dev_server = "off"` opts out. `leptoskit new --db` now means Postgres.

While `leptoskit dev` runs, changing the tables writes a migration for changes that keep every row and applies it to the development database before the app rebuilds; changes that would delete data, or that need a rename answered, write nothing and say which command to run. `[db] watch = "report"` only prints the changes.

Access policies can count linked rows: `cx.count(Member::query().filter(|m| m.team_id.eq(project.team_id))).gt(0)` lets the members of the team that owns a project see it, and `cx.exists(query)` is short for the same. Counts compile to correlated subqueries on Postgres and SQLite, apply the counted table's own read rules, and skip soft-deleted rows.
