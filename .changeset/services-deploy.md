---
"release": minor
---

# Services deploy beside the app

`leptoskit generate deploy` and `lk infra` now run an app's `[services]` next to its server, from the same image and release.

- **Dockerfile:** the release build copies each service's binary to `/app/services/<name>`, and the health check asks whichever port the container serves.
- **Compose:** each service is its own entry with its token, database, and volumes. A Caddy edge on the app's port sends public service paths to their service, and strips cookies for isolated ones. On Postgres, each service gets a database and role of its own on the first start.
- **Kubernetes:** each service is a Deployment and a Service with a Secret of its own, and an Ingress routes the public paths, next to `--split-roles` too. The Fly and systemd targets refuse services they can't run.
- **`lk infra`:** each service is a Kamal role on the primary host, reached privately through a network alias. Its public paths go through kamal-proxy with the prefix kept. Its secrets reach only it, through Kamal's `NAME:SECRET` aliases, and the app's environment moves to the app's own role. Releases create each service's Postgres database and role, the backups cover every service's database, and restores bring them back with the app's.
- **Connection budget:** `[infra.postgres]` sets `max_connections` and the app's `app_pool`. `lk infra plan` and `lk doctor` fail when the app's and the services' pools, counted through a deploy, don't fit.
