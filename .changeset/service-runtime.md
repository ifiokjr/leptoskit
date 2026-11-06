---
"release": minor
---

# A runtime for services that run beside the app's server

`leptoskit::service` is what a service, a separate process the app runs beside its server, needs at runtime. `ServiceEnv::from_env()` reads what `leptoskit dev`, `leptoskit serve`, and deploys start it with: its name, the address to bind, the token its trusted callers present, the app's private URL, its own database, and its volumes. `ServiceEnv::serve` serves its router with `/healthz` and `/readyz` and drains on `SIGTERM`, and `RequireToken` keeps a route to callers holding the token.

`leptoskit::service::main!(analytics_service, run)` writes the whole `main` of a service binary. It also links the library that declares the service's operations, without which the binary would answer every call with `404`. On the app's side, `service_url` and `service_token` read where a service is and how to call it.
