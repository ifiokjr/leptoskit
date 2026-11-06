---
leptoskit_deploy_console: major
---

# leptoskit_deploy_console: self-hosted deployments

`leptoskit_deploy_console` is the optional service behind `lk console`: one process that hosts the deployment dashboard, a durable operation queue, a scoped API for agents, and analytics history, backed by SQLite.

Run `lk console serve` on a trusted worker with Docker, OpenTofu, Kamal 2, Git, and SSH, then `lk console connect` from the app's checkout, and `lk infra plan`, `lk infra apply`, and `lk deploy` run through it. It reuses the CLI's OpenTofu and release engine, binds each apply to a reviewed plan, and keeps interrupted operations for explicit reconciliation. The `server` feature enables the service; the dashboard is a leptoskit component tree rendered on the server. It serves one owner's repositories and is not a multi-tenant executor.
