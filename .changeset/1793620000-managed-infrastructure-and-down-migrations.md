---
"release": minor
---

# Deploy, back up, and roll back on your own servers

Describe your servers in an `[infra]` table in `leptoskit.toml`, and `lk infra` (also `lk tofu`) and `lk deploy` take the app from there to running Linux hosts. They provision Hetzner or DigitalOcean servers behind a Cloudflare Tunnel, an R2 bucket that serves signed code push updates, and a private R2 or Backblaze bucket for backups, then run the app and Postgres in containers with Kamal:

```sh
lk infra generate
lk infra plan
lk infra apply
lk infra build --version release-001 --push
lk deploy --setup --version release-001
```

- `lk infra apply` applies only the plan you saved and reviewed, and refuses a configuration that changed since. `lk infra offers` lists current prices, and `[infra.budget]` stops a plan whose compute would cost more per month than you allow.
- Several app hosts update one at a time. An optional NATS relay for live connections and a bounded Valkey cache run on the private network; Postgres, NATS, and Valkey stay on one primary host.
- Scheduled backups are encrypted with Restic, restored into a throwaway database to check them, and pruned by your retention settings. `lk infra restore` keeps the previous database beside the restored one.
- Each image carries its migration history, which is checked before the app starts, and each release records its image digest. `lk deploy --rollback <tag>` checks both against the live database. Rolling back a schema change needs reviewed `down.sql` files, stopped writers, and `--rollback-schema --yes`; a failed schema change is never reversed automatically.
- `lk infra ci` writes a GitHub Actions workflow that builds an image for each push to `main`, tagged with its commit, and deploys it.

`lk db migration create` now writes a `down.sql` beside each new `migration.sql`; existing migrations and their checksums are left alone. `lk db rollback` and `Db::rollback` reverse applied migrations newest first, each in its own transaction, and refuse to start when a reverse is missing or still marked `-- leptoskit: irreversible`.

`leptoskit generate deploy` also deploys a client-rendered app whose backend and browser client are separate packages (`build.client_package`). A NATS relay URL that carries a user name and password now passes them to the client instead of to the connection address.
