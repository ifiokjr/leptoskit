---
"release": minor
---

# Service databases in `leptoskit db`, branches, and releases

Every `leptoskit db` command takes `--service <name>` to work on a service's own database instead of the app's: `leptoskit db migration create add_referrer --service analytics` reads the tables from the service's crate and writes the migration into its `migrations/` directory, or `migrations-postgres/` when the app is on Postgres, and `migrate`, `status`, `check`, `push`, `rollback`, `reset`, `studio`, and `release record` work the same way. Without `--url`, a command uses the database `leptoskit dev` gives the service, or `LEPTOSKIT_SERVICE_<NAME>_DATABASE_URL`; the app's `DATABASE_URL` never reaches a service command. A service installed from a registry or git ships its migrations, so the commands that write migrations refuse it. Unknown, remote, and database-less services fail with an error naming `[services.<name>]`.

On the development Postgres, a service now logs in as a role of its own, `<app>_service_<name>`, which owns its database and can't connect to the app's. On a git branch with its own app database, the service's database starts as a copy of the main branch's, and `leptoskit db branch list` and `prune` handle it with the app's.

`leptoskit infra` releases record each service database's migrations beside the app's. A release that changes a service's tables needs `--maintenance`, a service database that drifted from the history stops the release, and `lk deploy --rollback <tag> --rollback-schema --yes` stops the services and takes each service's database back to the release's tables with its down migrations. Releases recorded before this hold only the app's tables, so rolling back to one leaves the services' databases alone. Images built by `leptoskit infra build` carry an `io.leptoskit.services.migrations` label when the app has database-backed services.
