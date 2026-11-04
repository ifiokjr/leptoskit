---
"release": patch
---

# Package the hosted pilot for a Linux server

Build the broker, public site, console and documentation as one checksummed Linux release, with a systemd service that keeps persistent data separate from releases. Drain broker requests on termination so service restarts can finish active requests.

Use `[infra.external]` to provision a separately supervised service with the existing OpenTofu integration, including DigitalOcean, Cloudflare routing, the price guard and saved-plan protection, without requiring the managed Postgres/Kamal runtime.
