---
"release": minor
---

# Deploy the server to Docker, Compose, or Fly

A server-rendered app needed hand-written container files and its own shutdown logic before it could run anywhere but your machine. Now:

- **`leptoskit generate deploy --target <docker|compose|fly>`** writes a multi-stage `Dockerfile` (a `leptoskit build --release`, then an unprivileged, read-only runtime with the `LEPTOS_*` environment and a health check) and a `.dockerignore`, plus `compose.yaml` with Postgres or a SQLite volume, or `fly.toml` with readiness checks, a kill timeout past the grace period, and machines that stay up for long-lived connections. It reads everything from `leptoskit.toml` and `Cargo.toml`, keeps secrets in the environment, and prints the commands to run next. The files are yours: running it again changes nothing, and an edited file is kept unless you pass `--force`.
- **`/healthz` and `/readyz`** come from `leptoskit::server::Readiness`, a cheap, shared set of named checks your server passes once it's ready, such as after migrations. `/readyz` answers `503` naming the checks that fail.
- **`serve_listener_with` and `GracefulShutdown`** stop the server in the order a load balancer needs: on `SIGTERM` or Ctrl-C, fail readiness, keep serving for a drain delay (5 seconds by default), then stop accepting and finish the requests in flight within a grace period (20 seconds). `serve_listener` and `serve_router` behave as before.
- New projects' servers serve both probes, pass a `migrations` check once the database is migrated, and shut down gracefully.
