# Services: separate processes from one `leptoskit.toml`

Proposed 6 November 2026, against `main` at `8c93eff6`. §1–§2 describe the code as it is; §3 onward are proposals.

The question: how should an app run part of its backend outside the server-rendering server? The driving example is a full analytics stack (ingestion, storage, queries, a dashboard) that shouldn't share a process with the pages it measures. The answer here is **services**: other crates in the app's Cargo workspace, declared in the app's one `leptoskit.toml`, built and released with the app, and each run as its own process. They can share a host with the app, or not.

A **service plugin** is a plugin that supplies a service: the crate's implementation, its defaults, and how it connects to the app. Analytics is the first one.

This is deliberately not a workspace manifest over several leptoskit projects. One `leptoskit.toml` already governs a Cargo workspace with several crates, as `examples/huddle` does, so the config that lists the app's packages can also list its other processes. Running several independently released projects together is a different, larger problem, and nothing here needs it.

---

## 1. Why the analytics service shouldn't run in the server-rendering process

What ships today is in-process. `leptoskit_analytics` (see [analytics-and-infra-plugins.md](analytics-and-infra-plugins.md)) mounts `/_leptoskit/analytics` on the app's Axum router and keeps bounded minute windows in memory: by default at most 128 series and 15 windows. It exports deltas over OTLP/HTTP when `LEPTOSKIT_ANALYTICS_EXPORT_ENDPOINT` is set. That bound is what makes running in-process safe, but it also rules out a full analytics stack. Durable events, funnels, retention, and a query API need storage and CPU that shouldn't compete with rendering pages.

Moving the stack into its own process gives:

- **Isolation.** Abusive browser ingestion, a slow query, or a crash in analytics doesn't take CPU, memory, or uptime from page rendering. On a shared host, container limits enforce this.
- **Separate credentials.** The app can't read raw analytics data, and the analytics service can't touch the app's database.
- **Independent scaling** within one release: more analytics replicas without more app replicas.

What it costs is a second process to build, start, route to, and health-check. Services exist to make that cost small.

---

## 2. What exists to build on

| Piece                          | Where                                                                                  | What it gives services                                                                                                                                     |
| ------------------------------ | -------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Tooling plugins                | `[[plugins]]` and `LeptoskitPlugin` in `crates/leptoskit_plugin/src/lib.rs`            | Hooks for `configure`, `generate`, `before_build`, `after_build`, `before_serve`, and CLI commands. A service plugin adds one hook.                        |
| Finding and running the server | `pick_server`, `BuiltServer`, and `spawn_with` in `crates/leptoskit_cli/src/server.rs` | Picks a package's binary from cargo's JSON output, records it per project, and starts it with environment variables. This generalises to several binaries. |
| Roles                          | `crates/leptoskit_rpc/src/server/roles.rs`, `--split-roles`                            | The same binary running as several processes. Deploy generators already write one Deployment per role.                                                     |
| Container image                | `dockerfile` in `crates/leptoskit_cli/src/deploy.rs`                                   | One build stage running `leptoskit build --release`, copying one binary to `/app/server`. It can copy more.                                                |
| Hosts                          | `crates/leptoskit_cli/src/infra/runtime.rs`                                            | Kamal config with a `web` role and accessories (Postgres, Valkey, NATS, cloudflared) on the `kamal` Docker network.                                        |
| Edge                           | `crates/leptoskit_cli/src/infra/resources.rs`                                          | A Cloudflare tunnel whose ingress list routes the app's hostname. Ingress rules can also match a path.                                                     |
| Infra plugin catalog           | `[infra.plugins.analytics]` in `crates/leptoskit_config/src/infra.rs`                  | Versioned capability intent, app bindings, and secret references for analytics' embedded tier.                                                             |

The missing piece is a name for "another binary in this workspace that runs as its own process", which `dev`, `build`, `serve`, and the deploy generators all understand.

---

## 3. Configuration

A top-level `[services.<name>]` table, one per service:

```toml
[services.analytics]
package = "acme_analytics" # a member of the Cargo workspace with a binary
port = 3100 # where it listens, in dev and in the container
routes = [
	"/_leptoskit/analytics",
] # public path prefixes sent here instead of to the app
secrets = ["ANALYTICS_DATABASE_PASSWORD"]
env = { ANALYTICS_RETENTION_DAYS = "90" }
replicas = 1
limits = { cpus = 0.5, memory = "256m" }
```

| Key        | Meaning                                                                                                                                                                                                                                                                         |
| ---------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `package`  | Required. A Cargo workspace member. Its binary is chosen the way `pick_server` chooses the app's (the one named after the package, or the only one), or by `bin = "…"`.                                                                                                         |
| `port`     | Required. Unique, and not `serve.port`.                                                                                                                                                                                                                                         |
| `routes`   | Optional. Absolute path prefixes that reach this service at the app's public origin. They can't be `/` or overlap another service's routes, and they can't shadow what the app itself serves: `/pkg`, `/api`, `/live`, the auth routes. Without routes, the service is private. |
| `env`      | Plain values set on the service only.                                                                                                                                                                                                                                           |
| `secrets`  | Names resolved from the environment at start, as the app's runtime secrets are today. Each is given to this service only.                                                                                                                                                       |
| `replicas` | How many processes to run, per host in Kamal and per Deployment in Kubernetes. Defaults to 1.                                                                                                                                                                                   |
| `limits`   | Container CPU and memory limits. This is what enforces isolation on a shared host.                                                                                                                                                                                              |
| `plugin`   | Set by a service plugin (§6). Supplies defaults for every other key, and anything written here overrides them.                                                                                                                                                                  |

The name must match `[a-z][a-z0-9-]*`, so it maps cleanly to environment variable names. `[infra.services]` (cache and relay) keeps its meaning. The two tables are different, though the shared word is awkward; renaming `[infra.services]` to `[infra.accessories]` in a later schema version would remove the overlap.

Configuration changes regenerate the schema (`devenv shell schema:update`). Validation runs with the rest of `LeptoskitConfig::validate`, so `lk` reports a bad service before it builds anything.

---

## 4. The runtime contract

A service is any binary that follows this contract. `leptoskit_axum` gains a `service` feature that implements it in one call (`leptoskit::service::serve(router)`), so most services don't write it by hand.

**What a service receives:**

| Variable                    | Value                                                                           |
| --------------------------- | ------------------------------------------------------------------------------- |
| `LEPTOSKIT_SERVICE_NAME`    | its name                                                                        |
| `LEPTOSKIT_SERVICE_ADDR`    | the address to bind: `127.0.0.1:<port>` in dev, `0.0.0.0:<port>` in a container |
| `LEPTOSKIT_SERVICE_TOKEN`   | a secret its private routes require from callers                                |
| `LEPTOSKIT_PUBLIC_ORIGIN`   | the app's public origin, for origin checks on browser routes                    |
| its own `env` and `secrets` |                                                                                 |

**What the app and every other service receive, for each service:**

| Variable                         | Value                                                                                  |
| -------------------------------- | -------------------------------------------------------------------------------------- |
| `LEPTOSKIT_SERVICE_<NAME>_URL`   | the private base URL: `http://127.0.0.1:3100` in dev, a stable network name in deploys |
| `LEPTOSKIT_SERVICE_<NAME>_TOKEN` | the token above, so the caller can authenticate                                        |

**What a service must do:**

- Serve `/healthz` and `/readyz`, as every leptoskit server does today.
- Stop accepting work on `SIGTERM` and drain within the deploy's drain timeout.
- Treat public `routes` as untrusted. A route reached through the edge is the internet, whatever the hostname.

The private token is the first, simplest form of service identity. Service tokens from `leptoskit_idp` (`client_credentials`) can replace it later without changing the variables' names.

---

## 5. Commands

### 5.1 `lk build` and `lk serve`

`lk build` adds every service package to the cargo invocation that builds the app's server (`cargo build -p app -p acme_analytics …`), so shared dependencies compile once. The build record grows from `BuiltServer` to `BuiltServers`, which holds the app's binary and each service's. `lk serve` starts the services, waits for each `/readyz`, then starts the app.

### 5.2 `lk dev`

1. Builds the app and services together.
2. Starts each service with the §4 environment and waits for `/readyz`.
3. Starts the app with every `LEPTOSKIT_SERVICE_<NAME>_URL`.
4. When any service has `routes`, binds `serve.port` itself with a small reverse proxy. Each route prefix goes to its service, and everything else goes to the app on an internal port. The proxy must pass WebSocket upgrades through, because the app's live socket goes through it. Without routes, the app binds `serve.port` directly, as today.
5. On a file change, works out from `cargo metadata` which packages depend on the changed file. It rebuilds once and restarts only the affected processes.
6. Prefixes each output line with the process name.

### 5.3 `lk generate service <name>`

Writes `services/<name>/` as a new workspace member: a `Cargo.toml` that depends on `leptoskit` with the `service` feature, and a `main.rs` that serves a router with one example route. It also adds `[services.<name>]` with a free port. This is the "start a service" path for app authors, the same way `lk generate auth` starts sign-in.

### 5.4 Deploying

All services share one image and one release with the app:

- **Image.** The Dockerfile's build stage already runs `leptoskit build --release`. It also copies each service binary to `/app/services/<name>`. The image's default command stays `/app/server`, and each service runs the same image with a different command.
- **Release.** One image tag records the app and its services together, so a rollback restores them together. The app and its services always come from the same commit, so the calls between them can't drift. This is the main difference from microservices: one deployment unit with several processes, not independent deployments.
- **Compose.** One entry per service with `command: ["/app/services/<name>"]`, its own environment and limits, on the app's network. The entry's name is a stable address (`http://analytics:3100`).
- **Kubernetes.** One Deployment and one Service per service. This reuses the `--split-roles` generator, which already writes one Deployment per process kind. Public routes become Ingress paths.
- **Kamal (`lk infra`).** Each service becomes a Kamal role with `cmd: /app/services/<name>` and its limits as Docker options. Public routes become path rules in the tunnel's ingress list, placed before the catch-all rule for the app.

Internally the generators should loop over a list of processes instead of special-casing the app. A split role is a process of the app's binary, and a service is a process of another binary. Folding roles and services together this way keeps the generators from growing a second branch for each.

---

## 6. Service plugins

A service plugin is a `LeptoskitPlugin` with one more hook:

```rust
fn service(&self) -> Option<ServiceDefinition> {
	None
}
```

`ServiceDefinition` holds:

- the scaffold that `generate` writes: a crate whose `main` calls into the plugin's library, for example `leptoskit_analytics::service::main()`, so the implementation stays in a versioned crate and the app's `Cargo.lock` pins it;
- defaults for every `[services.<name>]` key;
- **bindings**: variables to set on the app when the service is present, for example `LEPTOSKIT_ANALYTICS_EXPORT_ENDPOINT` pointing at the service's private URL;
- **accessories**: containers the service needs, such as a database, with their volumes and secrets, which `lk infra` adds next to the existing ones;
- CLI commands under `plugin:<namespace>`, as plugins have today.

The user-facing configuration is one table:

```toml
[services.analytics]
plugin = "analytics"
```

`lk generate service analytics --plugin analytics` writes the crate and the table. Every other key comes from the plugin's defaults until the app overrides it.

The plugin can't run arbitrary code at deploy time or receive provider credentials, the same rule `[infra.plugins]` follows. It contributes data, the generators turn that into config, and a person reviews the result.

---

## 7. Analytics as the first service plugin

| Concern               | Embedded tier (today)                               | Service tier (proposed)                                                                                                       |
| --------------------- | --------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------- |
| Where it runs         | inside the app's server                             | `/app/services/analytics`, its own process and container limits                                                               |
| Browser ingestion     | `/_leptoskit/analytics/browser` on the app          | the same path, routed by the edge (or the dev proxy) to the service, so `browser.mjs` doesn't change                          |
| Backend measurements  | in-memory windows                                   | the app keeps its bounded in-process aggregator and exports deltas over the existing OTLP/HTTP path to the service            |
| Storage               | none; at most 60 minute windows                     | durable: a separate Postgres database and role on the existing accessory first; a column store as an optional accessory later |
| Queries and dashboard | the read API on the app; the console's panels       | the service's own query API and dashboard, behind a read key                                                                  |
| Privacy rules         | route templates, no raw URLs, GPC and DNT respected | unchanged; the same validation runs at the service's ingestion                                                                |

The embedded tier stays. Small apps keep `[infra.plugins.analytics]` and add nothing. Moving to the service tier means adding `[services.analytics]`, and the app's binding switches its export endpoint to the service automatically.

---

## 8. Delivery order

1. **Configuration, runtime, and local commands.** The `[services]` table and its validation, the schema, `leptoskit_axum`'s `service` feature, `lk build` and `lk serve`, `lk dev` with the proxy and per-package restarts, and `lk generate service`. Proof: an example with one private service and one routed service, plus a CLI test that starts both and checks that a routed request and a private call each reach the service.
2. **Deploy generators.** Dockerfile, Compose, and Kubernetes output, with insta snapshots, then Kamal roles and tunnel ingress in `lk infra`. Proof: a Compose run in CI, and a staged `lk infra` deploy with a service on the same host.
3. **The service plugin hook and analytics.** `ServiceDefinition`, the analytics service with durable storage, the app's binding, and the dashboard. Proof: browser, native, and backend measurements reach the service, and the app's measured latency doesn't change when ingestion is flooded.
4. **Later, only if needed.** A database per service through `leptoskit_db`, generated Rust clients for private calls (`lk rpc generate` already has a Rust target), `leptoskit_idp` service tokens, and deploying a service on its own schedule. That last one is the point where the shared-release model stops fitting and something like a workspace manifest becomes worth building.

## 9. Open questions

- **Stable addresses under Kamal.** Kamal names role containers per version, so a service needs a stable address on the `kamal` network for private calls and tunnel ingress. Check, against the pinned Kamal version, whether a network alias per role or kamal-proxy's path routing provides one. This is the first spike in step 2.
- **Public routes in Compose.** Compose has no edge. Either the generated Compose file adds a small proxy, or routes stay a Kamal and Kubernetes feature, and Compose users point a proxy of their own at the services.
- **Dev proxy versus server-side rendering.** The proxy sits in front of the server-rendering server in dev only. Check that it keeps streaming responses streamed, and doesn't buffer them.
- **Health and ordering.** Does the app wait for every service to be ready, or only for those it calls during startup? Starting all services first is simpler, but it makes the app's availability depend on analytics, which is the coupling services are meant to remove. The proposal: wait in dev, and in deploys let each process become ready on its own.

---

## Appendix: the X architecture

This note started from X's [Rebuilding the global town square from scratch](https://blog.x.com/en/engineering/rebuilding-the-global-town-square-from-scratch). That rebuild is mostly a consolidation: one request-aware rendering server, plus one Rust service for card images in the same repository. Services cover the second half; the card-image service would be one `[services.cards]` table. The first half needs rendering work that every leptoskit app benefits from, with or without services:

- **Route `load` and preloading.** The file router recognises `load` and `prerender` (`crates/leptoskit_router_codegen/src/exports.rs`) but doesn't call them yet, and there's no preloading on hover or focus.
- **Rendering for the audience.** A request classifier in `leptoskit_axum` that tells apart people, verified crawlers (checked by reverse DNS), and link-preview bots. Then a per-route policy: stream for people, send crawlers a complete document without hydration, and send link-preview bots only the head.
- **Rollout switches.** A switch the edge can set per request, so a new rendering path can be rolled out gradually.

Data in the first response already works: `leptoskit_query` scopes with `.with_ssr()` stream their data with the HTML.
