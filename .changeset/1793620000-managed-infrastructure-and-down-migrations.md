---
"release": minor
---

# Manage deployments, backups, and schema rollback

Add `[infra]`, `lk infra` (`lk tofu`), and `lk deploy` for Hetzner or DigitalOcean hosts behind Cloudflare Tunnel, R2 signed-update delivery, private R2 or Backblaze backups, and Postgres/Kamal container releases. Optional NATS relay and bounded Valkey cache use private networking; application hosts roll one at a time. Saved plans are checked before apply, image builds carry migration metadata checked before application startup, live prices can guard a compute budget, and a generated GitHub workflow builds and deploys tagged images. Shared reservations coordinate operators, releases, and backup jobs. Release history records registry digests and migration fingerprints on the primary host and in encrypted backups.

Install encrypted scheduled Restic backups with isolated restore checks, retention, health status, and explicit recovery that preserves the previous database. Image rollback checks the recorded digest and actual database history; schema-changing rollback requires reviewed reverse migrations, stopped writers, and explicit confirmation. Failed schema changes require recovery rather than an unsafe automatic image rollback. Persistent services remain on a single primary host.

Generate `down.sql` beside each new upward `migration.sql`, preserving existing migration directories and checksums. `Db::rollback` and `lk db rollback` reverse an applied prefix newest first, transactionally per migration. Missing reverses and unresolved destructive/data recovery stubs fail before work starts. SQLite rebuilds retain foreign-key checks and Postgres uses the migration advisory lock. Deployment file generation also accepts a client-rendered app with a separate backend and client package. Bind percent-decoded relay URL credentials explicitly to the NATS client and strip them from connector addresses.
