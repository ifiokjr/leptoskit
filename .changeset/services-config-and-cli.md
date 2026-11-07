---
"release": minor
---

# Services beside the app server

A `[services.<name>]` table runs another Rust binary beside the app's server, with its own port, its own database, and its own public paths on the app's origin. A service comes from a crate in the workspace (`package`), a crate `leptoskit` builds from a `path`, the registry (`crate`, `version`), or `git` (`rev`, `branch`, or `tag`), or it already runs elsewhere (`url`). The service crate describes itself in `[package.metadata.leptoskit.service]`: its `port`, whether it's `public`, extra `routes`, the `features` its binary needs, a `database` of its own with an optional `pool` size, `volumes`, `env`, `secrets`, and the variables it `bind`s on the app. The table in `leptoskit.toml` overrides any of them.

```toml
[db]
dialect = "sqlite"

[services.analytics]
package = "analytics_service"
```

`leptoskit build` builds the app's server and its workspace services in one cargo invocation, and installed services on their own, pinned by `services.lock`. `leptoskit dev` and `leptoskit serve` give each service its database (a SQLite file under `.leptoskit/services/`, or a database of its own on the app's Postgres server), its volumes, and a token; start each service and wait for `/readyz`; then start the app with `LEPTOSKIT_SERVICE_<NAME>_URL`, `LEPTOSKIT_SERVICE_<NAME>_TOKEN`, and the service's bindings. When a service is public, a proxy on the configured port sends `/_leptoskit/services/<name>` and its `routes` to it, streaming bodies and handing WebSocket upgrades through, and everything else to the app. A service whose `trust` is `isolated`, the default for installed and remote services, never receives the app's cookies and can't set them.

`leptoskit services` lists the resolved services, their databases, public paths, volumes, and the variables the app receives, without printing a token.
