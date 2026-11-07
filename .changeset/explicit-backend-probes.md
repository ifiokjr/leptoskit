---
"release": patch
---

# Explicit backend selection and readiness probes

`build.backend` explicitly selects an API backend or a static app. Build, serve and deployment generation honor it; CSR apps no longer need a separate client package to deploy a backend. Without an explicit choice, Cargo binary targets determine backend presence.

`serve.probes` declares the `/healthz` and `/readyz` lifecycle instead of guessing from words in Rust source. New projects enable it. Existing managed apps must implement the readiness routes and set `serve.probes = true` before deploying. The declaration is checked by deployment health probes at runtime; it is not evidence that a server is healthy.
