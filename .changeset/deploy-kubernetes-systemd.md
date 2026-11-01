---
"release": minor
---

# `lk generate deploy --target kubernetes|systemd`

The deploy generator gains two targets. `kubernetes` writes a Deployment, Service, and `PodDisruptionBudget` with `/readyz` and `/healthz` probes, a preStop drain, a read-only root filesystem, and a `PersistentVolumeClaim` for SQLite (or two replicas and a Secret reference for Postgres). `systemd` writes a hardened `<app>.service` with a 30-second stop, an `EnvironmentFile`, a state directory for SQLite, and the sandboxing flags a public server wants. Both share the generated `Dockerfile` and are documented in "Deploying a server".
