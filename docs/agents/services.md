# Services: separate processes from one `leptoskit.toml`

Proposed 6 November 2026, against `main` at `8c93eff6`. This is the design and the contract the implementation follows. The status of each part is in §11.

**Services** let an app run part of its backend outside the server-rendering server. A service is a Rust binary with:

- its own process and port, on the same host as the app or not;
- **its own data:** a database on the app's database engine;
- **its own RPC operations,** including live queries over its own data, which the app's screens and server call with the same typed client they use for the app's own operations.

A service reaches the app's data only through the app's ordinary request/response operations (§6), never through the app's database or its live queries.

The app declares its services in its one `leptoskit.toml`. A service can be:

- **internal:** a crate in the app's Cargo workspace;
- **installed:** a crate from a path, a registry, or git, which `lk` builds;
- **remote:** already running elsewhere, so `lk` only connects the app to it.

A **service plugin** is a crate that ships a ready-made service and describes itself in its Cargo manifest. `examples/analytics` is the reference service plugin. A wider, PostHog-style analytics plugin will follow the same shape.

This is deliberately not a workspace manifest over several leptoskit projects. One `leptoskit.toml` already governs a Cargo workspace with several crates, as `examples/huddle` does, so the config that lists the app's packages can also list its other processes.

---

## 1. Why

The motivating case is analytics. What ships today is in-process: `leptoskit_analytics` (see [analytics-and-infra-plugins.md](analytics-and-infra-plugins.md)) mounts `/_leptoskit/analytics` on the app's router and keeps bounded minute windows in memory. A full product-analytics stack (durable events, funnels, retention, live dashboards) needs storage and CPU that shouldn't compete with rendering pages. It also shouldn't be able to take the app down, and it shouldn't hold the app's database credentials.

---

## 2. Configuration

```toml
# An internal service: a crate in this workspace.
[services.analytics]
package = "analytics_service"

# An installed service: built by `lk` from outside the workspace.
[services.search]
crate = "acme-search-service"
version = "0.4"
# or: git = "https://github.com/acme/search", rev = "…"
# or: path = "../search"

# A remote service: already running elsewhere. `lk` connects the app to it and runs nothing.
[services.billing]
url = "https://billing.internal.example"
```

Each `[services.<name>]` table has exactly one source: `package`, `path`, `crate` (with an optional `version`), `git` (with at most one of `rev`, `branch`, or `tag`), or `url`. The name must match `[a-z][a-z0-9-]*`, and `volume` and names starting `volume-` are reserved, because `LEPTOSKIT_SERVICE_VOLUME_*` names a service's volumes. In environment variable names it's uppercased, with `-` becoming `_`. With `git`, a `crate` key picks the package inside the repository.

The table can also override anything the service's manifest (§3) declares:

| Key        | Meaning                                                                                                                                                                                         |
| ---------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `bin`      | Which binary to run, when the package has several.                                                                                                                                              |
| `features` | Cargo features to enable when building the service binary.                                                                                                                                      |
| `port`     | Where the service listens. Unique, and not `serve.port`.                                                                                                                                        |
| `public`   | Whether browsers and native apps can reach it at `/_leptoskit/services/<name>/` on the app's origin. Defaults to `false`.                                                                       |
| `routes`   | Extra public path prefixes, such as `/_analytics`. Requires `public`. Can't be `/`, overlap another service's prefixes, or shadow the app's own paths (`/pkg`, `/api`, `/live`, `/_leptoskit`). |
| `pool`     | The service database's pool size, passed as `LEPTOSKIT_SERVICE_DATABASE_POOL`.                                                                                                                  |
| `database` | Whether the service gets its own database (§3).                                                                                                                                                 |
| `env`      | Plain values set on the service.                                                                                                                                                                |
| `secrets`  | Names passed through from the environment to the service only.                                                                                                                                  |
| `volumes`  | Named persistent directories the service owns (§3).                                                                                                                                             |
| `bind`     | Variables set on the app when this service is present (§4).                                                                                                                                     |

`trust` is set only in `leptoskit.toml`: `"first-party"` (the default for `package`) or `"isolated"` (the default for `path`, `crate`, `git`, and `url`). Isolated services get `Cookie` stripped from requests and `Set-Cookie` from responses on their paths (§10).

`forward_identity` is also set only in `leptoskit.toml`, never by a manifest: whether the app tells the service who is signed in (§5, "Forwarding the signed-in person"). It defaults to `true` for first-party services and `false` for isolated ones, so an installed or remote service learns who anyone is only when the app's author says so.

The merge rule: a value in `leptoskit.toml` replaces the manifest's, and the `env` and `bind` maps merge key by key, with `leptoskit.toml` winning. `env`, `secrets`, and `bind` can't name a variable `lk` sets itself, and `bind` placeholders are limited to `{url}`, `{token}`, and `{public_path}`.

A remote service takes only `url`, `bind`, `trust`, and `forward_identity`. It can't be `public`: browsers reach public services at `/_leptoskit/services/<name>/` on the app's origin, and neither the dev proxy nor any deploy's edge forwards those paths to another origin yet, so a public remote service would answer 404. The app's server calls it at its `url`. Its token comes from the operator's environment as `LEPTOSKIT_SERVICE_<NAME>_TOKEN`.

---

## 3. The service manifest and its data

A service crate describes itself in `[package.metadata.leptoskit.service]`, the same way runtime plugins use `[package.metadata.leptoskit.runtime]`. It's data only, so `lk` never runs plugin code to learn what a service needs.

```toml
[package.metadata.leptoskit.service]
name = "analytics"
port = 3100
public = true
features = ["service"]
database = true
volumes = ["exports"]
env = { ANALYTICS_RETENTION_DAYS = "90" }

[package.metadata.leptoskit.service.bind]
ANALYTICS_PUBLIC_PATH = "{public_path}"
```

`name` is required whenever a manifest exists, and `ResolvedService::merge` fails when the `[services.<key>]` table uses another key, naming both. An operation's `service = "…"` is fixed at compile time, while the table key isn't, so a mismatch would send every call to a prefix and variables the process doesn't answer. At runtime, `ServiceEnv::expect_name` gives a service binary the same check (`examples/analytics` uses it).

### A database per service, on the app's engine

A service with `database = true` gets a database of its own on the engine the app's `[db] dialect` names, or SQLite when the app has no `[db]`. It plugs into the app's existing database infrastructure rather than declaring new infrastructure:

|          | SQLite app                                                                                                                    | Postgres app                                                                                                                                    |
| -------- | ----------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------- |
| In dev   | `.leptoskit/services/<name>/<name>.sqlite3`                                                                                   | a database `<app>_service_<name>` on the dev Postgres server                                                                                    |
| Deployed | a file on a volume of its own (Compose `<name>-data`, Kubernetes a PersistentVolumeClaim `<app>-<name>-data`), so one replica | a database and a login role of its own, both `<app>_service_<name>` (`leptoskit_service_<name>` under `lk infra`), on the app's Postgres server |

The `_service_` segment keeps service databases and roles out of the names leptoskit uses itself: the managed `leptoskit` database, the `leptoskit_admin` superuser, `app__<branch>` branch databases, and recovery's `lk_*` copies. Without it, a service named `admin` would have been given `leptoskit_admin`, and the setup's `ALTER ROLE … PASSWORD` would have taken over the superuser. A name past Postgres's 63 bytes is shortened with a hash of the service's name, as branch databases are (`services::postgres_database`).

The service receives its URL as `DATABASE_URL`. It embeds its migrations with `leptoskit::db::migrations!` and applies them at startup with `Db::migrate`, as `huddle_server` does, so installed services work without the CLI generating anything. A service that supports both engines ships both directories, as huddle does with `migrations/` and `migrations-postgres/`.

Each service gets its own database, not a schema or tables inside the app's, for these reasons:

- **Jobs.** A job worker claims any due job in `rpc.jobs`, and fails, with a retry, a job whose name its build doesn't know (`rpc/server/jobs/worker.rs`). Two processes sharing one `rpc` schema would claim and fail each other's jobs.
- **SQLite live queries.** Versions are stamped correctly across processes, but only commits made through a process wake that process's change feed. A process would see another process's writes only when it next writes itself.
- **Isolation.** On Postgres, a role per service means the service can't read the app's tables and the app can't read the service's.
- **Ownership.** Each service owns its migration history, its `live` and `rpc` schemas, and its backups.

The first two are fixable, by filtering job claims to registered names and by polling the SQLite feed. Both fixes are worth making on their own, but neither changes the decision.

### Other resources

- **Volumes:** named persistent directories, found at `LEPTOSKIT_SERVICE_VOLUME_<VOLUME>`. In dev a volume is `.leptoskit/services/<name>/<volume>/`.
- **In-memory storage** is the service's own memory and needs nothing declared.
- **A shared cache.** Later, a service can opt into the app's existing `[infra.services] cache` (Valkey) with its own user, limited to keys under `<name>:`.
- **Containers.** Arbitrary containers, such as a column store, are out of scope for now.

---

## 4. The runtime contract

**Variables a service receives:**

| Variable                            | Value                                                                                         |
| ----------------------------------- | --------------------------------------------------------------------------------------------- |
| `LEPTOSKIT_SERVICE_NAME`            | its name                                                                                      |
| `LEPTOSKIT_SERVICE_ADDR`            | the address to bind: `127.0.0.1:<port>` in dev, `0.0.0.0:<port>` in a container               |
| `LEPTOSKIT_SERVICE_TOKEN`           | its secret: trusted callers present it to the service, and the service presents it to the app |
| `LEPTOSKIT_PUBLIC_ORIGIN`           | the app's public origin                                                                       |
| `LEPTOSKIT_APP_URL`                 | the app server's private base URL (§6)                                                        |
| `DATABASE_URL`                      | its own database, when it has one                                                             |
| `LEPTOSKIT_SERVICE_DATABASE_POOL`   | its database pool size, when `pool` is set                                                    |
| `LEPTOSKIT_SERVICE_DRAIN_MS`        | how long it keeps accepting after `SIGTERM`; `lk dev` sets 0, deploys leave the default (5 s) |
| `LEPTOSKIT_SERVICE_VOLUME_<VOLUME>` | each volume's directory                                                                       |
| its `env` and its `secrets`         |                                                                                               |

**Variables the app receives, for each service:**

| Variable                            | Value                                                                                  |
| ----------------------------------- | -------------------------------------------------------------------------------------- |
| `LEPTOSKIT_SERVICE_<NAME>_URL`      | its private base URL, with no trailing slash                                           |
| `LEPTOSKIT_SERVICE_<NAME>_TOKEN`    | its token                                                                              |
| `LEPTOSKIT_SERVICE_<NAME>_IDENTITY` | `forward`, only when `forward_identity` is on (§5)                                     |
| each `bind` entry                   | with `{url}`, `{token}`, and `{public_path}` (`/_leptoskit/services/<name>`) filled in |

**What a service does:**

- Serves `/healthz` and `/readyz`.
- Serves everything public under `/_leptoskit/services/<name>/`, plus any extra `routes`. The proxy and the edge forward paths unchanged.
- Drains on `SIGTERM`.
- Treats public paths as the internet, and trusts a caller only when it presents `Authorization: Bearer <LEPTOSKIT_SERVICE_TOKEN>`, or, as a signed-in person, a forwarded-identity token minted for it (§5).

`leptoskit_axum::service` implements the contract: `ServiceEnv::from_env()`, the public prefix, the app's URL, volumes, a token check, and `serve`.

**What a service inherits in dev.** `lk dev` and `lk serve` start a service with a cleared environment: only what a program needs to run (`PATH`, `HOME`, `USER`, `LOGNAME`, `TMPDIR`/`TMP`/`TEMP`, `LANG`/`LANGUAGE`/`LC_*`, `TZ`, `RUST_LOG`/`RUST_BACKTRACE`/`RUST_LIB_BACKTRACE`, `SSL_CERT_FILE`/`SSL_CERT_DIR`, the proxy variables, `TERM`/`NO_COLOR`/`CLICOLOR*`, and the Windows system variables; `services::run::INHERITED`), the contract's variables, its `env`, and its declared `secrets`. Anything else in the developer's shell, such as the app's own `DATABASE_URL` or a cloud token, never reaches it. The app keeps the whole environment.

**Starting and stopping.** Before starting a service, and the app, `lk` checks its port is free: a process an earlier session left running would otherwise answer `/readyz` in the new one's place. After the readiness check passes, it confirms the child it started is still the one running. Stopping sends `SIGTERM`, then `SIGKILL` after a grace period: 25 s under `lk serve` (the runtime's 5 s drain and 20 s grace) and 5 s under `lk dev`, where services skip their drain. Windows has no `SIGTERM`, so there they're killed at once.

---

## 5. Operations owned by a service

A service's operations live in a library crate, typically the service crate's own `lib`, which both the service binary and the app depend on:

```rust
/// Records one event.
#[rpc::mutation(service = "analytics")]
pub async fn capture(cx: &rpc::MutationCx, event: String, distinct_id: String) -> rpc::Result<()> { … }

/// Counts per event: a live query over the service's own `events` table.
#[rpc::query(service = "analytics")]
pub async fn summary(cx: &rpc::QueryCx, days: u32) -> rpc::Result<Summary> { … }
```

The macro gives the operation `const SERVICE: Option<&'static str> = Some("analytics")`. The body and its registration are kept only when the crate defining the operation has its own `service` feature on. Elsewhere, including in the app's server, the macro emits the typed arguments struct and a stub `run` that is never called.

| Where the call is made            | Calls                                                                                                                                          | Live queries                                                                                                                 |
| --------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------- |
| the browser                       | a same-origin `POST /_leptoskit/services/analytics/api/<op>`                                                                                   | the service's own socket at `/_leptoskit/services/analytics/live`, opened the first time a screen watches one of its queries |
| a native app                      | the configured origin with the same path                                                                                                       | the same, at the configured origin                                                                                           |
| the app's server, while rendering | `POST $LEPTOSKIT_SERVICE_ANALYTICS_URL/_leptoskit/services/analytics/api/<op>` with `Authorization: Bearer $LEPTOSKIT_SERVICE_ANALYTICS_TOKEN` | the first value only, over the same HTTP route, as live queries render today                                                 |

A live query owned by a service runs on the service's hub, over the service's database. It works the way the app's live queries do, with change capture, read sets, policies, and resume. A screen can watch the app's live queries and a service's live queries at the same time, each over its own connection. One live query can't span the app's data and a service's data.

`rpc::server::ServiceCallers` wraps an `Identify`. A request carrying a known service token is identified, as anonymous, with a caller name: `ServiceCallers::for_service(inner)` reads `LEPTOSKIT_SERVICE_TOKEN` and names the caller `"app"`; `ServiceCallers::for_app(inner)` reads every `LEPTOSKIT_SERVICE_<NAME>_TOKEN` and names each `"service:<name>"`; `ServiceCallers::new(inner).caller(token, name)` sets one by hand. Comparison is constant-time. Operations read the caller from `cx.identity().caller`. `for_service` also accepts the people the app forwards to the service `LEPTOSKIT_SERVICE_NAME` names, as `person:<id>` (`.forwarded_identity(name, token)` by hand); see below.

A service calling its own operations (the installed server's `Server::service_name()` equals the operation's `SERVICE`) runs them in process, as the app does with its own. A server that doesn't serve operations can still call services with the `services` feature (`rpc-services` through the facade), which `server` implies.

How a service builds its server:

```rust
let rpc = Server::new(db).service("analytics").identify(ServiceCallers::for_service(Anonymous));
let live = rpc.live().origin(public_origin); // the live hub inherits the scope
let app = Router::new().nest(&rpc::service_prefix("analytics"), rpc.router().merge(live.router()));
```

The exported spec filters the same way: `SpecOptions::service(name)` (or `LEPTOSKIT_RPC_SPEC_SERVICE`) exports one service's operations, with paths under its prefix and `live_path` moved there too. Without it, the spec holds only the app's.

### What enforces the boundary

A spike against `main` at `8c93eff6` (two throwaway crates: an operations library with a `service` feature, and an app server depending on it) showed that Cargo features alone can't keep a service's operations out of the app:

1. **Today the app compiles and serves service operations.** `#[rpc::query]` keeps the body whenever `leptoskit_rpc_macros/server` is on, and the app's server turns that on. So a body that uses service-only code breaks the app's build. A self-contained body compiles into the app, registers there, and would be served at the app's own `/api/<name>`, running against the app's database.
2. **Workspace builds unify features.** In `cargo build --workspace` or `cargo test --workspace` (what CI runs), the service's `service` feature is on for the app too. A `#[cfg(feature = "service")]` gate is then true in the app as well, and the body compiles and registers there.
3. **Registrations need linking.** `inventory` registrations exist only in crates the linker keeps. A service binary that never references its own library has no operations, and every call returns 404. `extern crate <lib>;` fixes it.
4. **Nesting works.** Under `Router::nest("/_leptoskit/services/<name>", …)`, the RPC routes, the live WebSocket (101), and server-sent events (200) all work, and the live origin check still refuses a foreign origin (403).

So the boundary is enforced in three layers:

- **The macro gate.** Bodies and registrations are kept only under the defining crate's own `service` feature. Elsewhere the macro emits a typed stub. This is what lets the app compile without the service's server-only dependencies.
- **Registrations scoped to their service.** Each `Registration` records `service: Option<&'static str>`. `Server::new(db)` serves only operations with no service, and `Server::new(db).service("analytics")` serves only analytics'; the built-in `rpc.*` operations are served by every server. `registrations()` checks name uniqueness per service; `registrations_in(scope)` and `registration_in(scope, name)` look within one scope, and `registration(name)` means the app's. The live hub and the in-process client look up only their server's scope, and the exported spec (`#[api]`, `lk rpc generate`) filters the same way. This keeps a unified build correct.
- **Routing by `SERVICE`.** The client sends every service-owned operation to its service, so the in-process client never runs one, even when its body is linked into the app.

With all three layers, building the app and its services in one cargo invocation is safe. It's also cheaper: separate invocations recompiled `leptoskit_db` and `leptoskit_rpc` for the second feature set. `lk` builds them together, with the services' `features` on.

The runtime's `leptoskit::service::main!(analytics_service, …)` references the library, so the link can't be forgotten.

**Jobs, cron schedules, streams, and channels are scoped the same way.** `service = "…"` on `#[rpc::job]`, `#[rpc::stream]`, and `#[rpc::channel]` (a cron schedule inherits its job's) emits `SERVICE`, keeps the body and registration under the declaring crate's `service` feature, and registers with the service's name. Every lookup is scoped:

- job workers take their scope from `Jobs::service`, or from the installed `Server` when they start;
- the scheduler enqueues only its own scope's schedules;
- the live socket finds streams and channels in its own scope.

Enqueueing a job that another scope owns fails with an error naming the service that owns it, so there's no cross-process enqueueing. A service's operation can't take an `rpc.*` name, and `Registration::is_builtin` counts only the `rpc.*` operations `leptoskit_rpc` registers itself. One gap remains: a service process that starts `Jobs` before installing its `Server`, and doesn't call `.service(…)`, runs only the app's scope.

**What a call carries.** The invariant: a call to a service carries what the person's browser would send, and no more, so a server-rendered page holds only what that person's browser could fetch, and hydration sees the same results.

- **From the server, while it renders.** Calls never carry the service token. When the app forwards identity to the service, they carry the forwarded-identity token of the person the page is rendered for (the installed server identifies the request leptos provides, then mints, as below); otherwise, and outside a request, they're anonymous. Only `rpc::client::call_as_app` (server only, outside wasm) adds `Authorization: Bearer $LEPTOSKIT_SERVICE_<NAME>_TOKEN`. Query and live hooks never call as the app, so nothing app-only is rendered into HTML.
- **From browsers and native apps.** Requests carry none of the app's own credentials unless `ClientConfig::service_credentials(name, …)` names the service, and a service's 401 never renews the app's credentials. Without that, each service's client signs with an `IdentityToken` for that service alone: the forwarded-identity token when the app gives one, and nothing otherwise. A service's live client has no offline queue: its mutations go over HTTP while its socket is down.

### Forwarding the signed-in person

A service that keeps per-person data (a PostHog-style "my events", per-person live queries) needs to know who is calling. The app tells it with a short-lived token it mints for one service:

```text
lkid1.<claims>.<signature>
claims    = base64url(JSON { "aud": service, "sub": person, "iat": secs, "exp": secs })
signature = base64url(HMAC-SHA256(key, "lkid1." + claims))
key       = HKDF-SHA256(salt = "leptoskit/service-identity/v1", ikm = the service's token, info = the service's name)
```

- **Who.** `sub` is the request's `leptoskit_db::Actor` UUID, which `leptoskit_auth` sets for every signed-in request. Nothing else about the person (session, wallets, assurance level) goes to the service. A request with no `Actor`, including a `service:*` caller, gets no token.
- **Minting.** `rpc::server::service_identity_token(service, identity)` mints for `IDENTITY_LIFETIME` (5 minutes) when the app's `LEPTOSKIT_SERVICE_<NAME>_IDENTITY` is `forward`, and refuses with `Forbidden` otherwise. Every server answers the built-in action `rpc.service_token { service }` with it: `Option<ServiceToken { token, expires_in }>`, `null` for nobody. It's an action, so no transaction and no receipt: a token is never stored.
- **The key.** It comes from the token `lk` already deploys to both sides, so no new secret goes through Compose, Kubernetes, Kamal, Docker, or `lk dev`; only the `_IDENTITY` flag does, as a plain value beside `_URL`. The HKDF salt separates it from the bearer token and from any later key made from it; the name as info, and `aud`, bind it to one service. A token minted for one service fails the MAC at every other, even one deployed with the same token.
- **Verifying.** `ServiceCallers` with a key treats a bearer starting `lkid1.` as a forwarded identity and refuses (401) a token whose MAC, `aud`, or times don't hold: `iat` up to 30 s ahead and `exp` up to 30 s past, for clock skew; a token claiming to last over 15 minutes is refused whatever its MAC says. The MAC is compared in constant time (`hmac`'s `verify_slice`). An accepted one becomes `Identity::new(Context::new().with(Actor).with(ForwardedIdentity { person, expires_at }))` with the caller `person:<id>`: distinguishable from `"app"`, and not `service:*`, so the service's live socket takes it. Its lease ends at `exp` and asks for a refresh a minute before, with the caller as continuity, so a renewed token keeps the connection's results.
- **The browser and native client.** `IdentityToken` asks `rpc.service_token` through the app's own client (cookie or bearer), keeps the token until a minute before it runs out, asks again on a 401 or a live refresh, and signs only that service's HTTP calls and live socket (the `auth` frame, or the upgrade's `Authorization` header; never a URL). When the app gives no token (nobody signed in, not forwarded, or the app unreachable or too old to know the action) the calls go anonymously, and it waits 5 minutes before asking again. `rpc::client::reauthenticate()`, which `leptoskit_auth`'s client calls on every sign-in and sign-out, forgets every token first, then reauthenticates the services' sockets, which ask for the new person's token or reconnect anonymously.

**Threat model.**

| Threat                                                     | What stops it                                                                                                                                                                           |
| ---------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| A service replays a token at another service or at the app | The key and `aud` are per service, so another service's MAC fails; the app never accepts `lkid1.` tokens (`for_app` has no key)                                                         |
| A browser forges or edits a token                          | HMAC-SHA256 under a key only the app and the service hold                                                                                                                               |
| A stolen token                                             | It works at one service for at most 5 minutes (15 minutes whatever it claims), and only as that person                                                                                  |
| Signing out                                                | The client forgets its tokens and asks for no more; the service's socket closes at `exp` unless renewed, and renewing fails once signed out. A handed-out token lives out its 5 minutes |
| An isolated service learning who people are                | The app mints only with `forward_identity`, off by default for isolated services                                                                                                        |
| Tokens in URLs, logs, or HTML                              | Headers and `auth` frames only; `ServiceToken`, `IdentityKey`, and `ServiceCallers` print no secret; rendering keeps the token in the request, never in what it renders                 |
| Clock skew between the app and the service                 | 30 s of leeway both ways                                                                                                                                                                |
| Key reuse                                                  | HKDF with a versioned label and the service's name                                                                                                                                      |

---

## 6. App data, from a service

A service reaches the app's data only through the app's ordinary request/response operations:

- **The client.** The service depends on the app's generated Rust client (`lk rpc generate`, target `rust`). It doesn't depend on the app's operations crate, which would compile the app's operation bodies into the service.
- **The address.** The client calls `LEPTOSKIT_APP_URL` with `Authorization: Bearer $LEPTOSKIT_SERVICE_TOKEN`.
- **Identity.** On the app, `ServiceCallers` holds every service's token and identifies the caller as `service:<name>`. The app's operations and policies decide what that caller may read and write. By default it's anonymous, with a caller name. The app holds no forwarded-identity key, so a person's token for a service never signs anything in at the app.
- **No live subscriptions.** A service doesn't subscribe to the app's live queries, and the app's live socket, server-sent events, and frames answer `403` to `service:*` callers. This includes a connection that opens anonymously and then sends a service's token in an `auth` frame: the session refuses it with the same 403 and closes. A service that needs fresh app data asks for it, or the app tells it by calling one of the service's operations.

Subscribing to app data (live queries for current views, a change-log stream for every change) is a later extension, not part of this design.

---

## 7. Commands

- **`lk build`** builds the app's server and every internal service in one cargo invocation, with the services' `features` on (§5 explains why that's safe). An installed service is resolved with `cargo metadata` over a generated manifest in `target/leptoskit/services/<name>/` and built from the source cargo fetched. All registry and git services resolve through one stub manifest in `target/leptoskit/services/fetch/`; its `Cargo.lock` is kept in the project at `services.lock`, copied back before each resolve, and release builds use `--locked` and fail without it, so an installed service can't change between two builds of the same commit. Installed services build with `--target-dir target/leptoskit/services/target`. Service binaries are recorded in `services-<hash>.json` beside the app's `server-<hash>.json`, so old build records still load.
- **`lk dev`** creates each service's database and volumes, then starts each service with the §4 environment and waits for `/readyz`. Then it starts the app with the bindings and the services' tokens. When any service is public, `lk dev` binds `serve.port` itself and runs a reverse proxy there. Each service's public prefixes go to that service, and everything else goes to the app on an internal port. The proxy keeps streaming responses streamed and passes WebSocket upgrades and server-sent events through. A rebuild restarts the services and the app.
- **`lk serve`** does the same with the last build, in the production environment, and stops its children on `SIGINT` or `SIGTERM`.
- **`lk services`** lists the resolved services: their source, port, trust, database (engine, location, pool), public paths, volumes, binding names, and features. `--json` gives the same as JSON. It never prints tokens or credentials.

In dev, tokens are made once per `lk dev` session (32 random bytes, hex) and kept across rebuilds; the proxy answers `502` naming the side that didn't answer, and strips cookies for isolated services. Services start with a cleared environment and receive only their declared `secrets` (§4), so a secret is confined to the services that list it.

- **`lk db … --service <name>`** (later) targets a service's database, and generates a service's migrations from its `#[db::table]` structs.

## 8. Deploying

All of an app's services share its release. `lk build` in the image's build stage builds them; the Dockerfile installs each process service's binary at `/usr/local/lib/leptoskit/services/<name>` and copies that directory to `/app/services`. The default command stays `/app/server`, each service runs the same image with its own command, and the health check reads its port from `LEPTOSKIT_SERVICE_ADDR` when set. A rollback restores the app and its services together. With no services, every generated file is unchanged.

| Target             | A service becomes                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                         |
| ------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Compose            | an entry running `/app/services/<name>`, with the §4 variables (container addresses, tokens as `${…}` references), its volumes, and a `/readyz` health check. The app's `depends_on` waits only for the services to start (`service_started`), so an unhealthy service never blocks `compose up`. Public paths go through a generated Caddy edge (`flush_interval -1`, WebSockets native, `header_up -Cookie`/`header_down -Set-Cookie` for isolated services; needs Compose 2.23.1+ for inline configs), and the app publishes no port itself. On Postgres an inline init creates a role and database `<app>_service_<service>` per service on an empty volume.          |
| Kubernetes         | a Deployment, Service, and Secret (`envFrom`) per service, with `/readyz` and `/healthz` probes; one replica with `Recreate` when it has a SQLite database or volumes. An Ingress lists public service prefixes, then `/live` under `--split-roles`, then `/`. An Ingress can't strip cookies, so generation refuses a public `isolated` service, naming the ways out (private, `first-party`, or Compose).                                                                                                                                                                                                                                                               |
| Kamal (`lk infra`) | a role `service-<name>` on the primary host with `cmd`, `options.network-alias: <infra.name>-<service>`, and only its own secrets through Kamal's `NAME:SECRET` aliases; the web role gets `<infra.name>-app`, which services use as `LEPTOSKIT_APP_URL`. Public paths use kamal-proxy `path_prefix` routing on the app's host. Every release and `lk infra prepare` create or update each service's Postgres role and database idempotently (passwords over SSH stdin); `recovery.py` dumps, verifies, and restores every service database with its owner, then revokes `PUBLIC`'s `CONNECT` on the app's and every service's database again, as a release's setup does. |
| Docker             | the next steps print, per service, a complete `docker run` on a shared network (`docker network create <image>`): the contract's variables with container names as addresses, its token from `${LEPTOSKIT_SERVICE_<NAME>_TOKEN}`, its database (a volume and SQLite URL, or `${LEPTOSKIT_SERVICE_<NAME>_DATABASE_URL}` on Postgres), volumes, `env`, and `secrets`; the app's run gains each service's URL, token, and `bind` values. Nothing routes public paths here, which the steps say.                                                                                                                                                                              |
| Fly, systemd       | refused with an error naming Compose, Kubernetes, and `lk infra`.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                         |

Kamal claims, checked against Kamal v2.10.1 (the version `lk infra` runtime config was validated with, see [infrastructure-exploration.md](infrastructure-exploration.md)):

- `env.secret` entries of the form `NAME:SECRET` set `NAME` from the secret `SECRET`: `lib/kamal/configuration/docs/env.yml` ("Aliased secrets", `DB_PASSWORD:MAIN_DB_PASSWORD`) and `Kamal::Configuration::Env#extract_alias` (`key.split(":", 2)`).
- A role's `options` map becomes `docker run` flags through `Kamal::Utils.optionize` (`--<key> <value>`), appended after `--network kamal` in `Kamal::Commands::App#run`, so `network-alias` becomes `--network-alias`, Docker's stable DNS name on that user-defined network.

Limits: network aliases resolve on one Docker host only, so `lk infra plan` and releases refuse process services when `instances > 1`. `lk infra` refuses public `isolated` services (a Cloudflare transform ruleset would take over the zone's single entry point and drop existing rules); remote services can't be public at all (§2). `--rollback-schema` and the release history checks cover only the app's database.

**Connection budget.** `[infra.postgres]` sets `max_connections` (default 100; a raised value starts Postgres with `-c max_connections=N`) and `app_pool` (default 8). `lk infra plan`, releases, and `lk doctor` count (hosts + 1 for the deploy overlap) × (app pool + 1 for live), plus 2 × (pool + 1) per database-backed service, plus 5 reserved, and fail with the breakdown when it doesn't fit.

## 9. Analytics as the reference service plugin

`examples/analytics` has two crates:

- **`analytics_service`.** A library with the `capture` mutation, a live `summary` query, an app-only `recent` query, and a live `mine` query of the signed-in person's own events, all owned by the `analytics` service, and a binary behind its `service` feature. `capture` records a forwarded person's events under `person:<id>`, which an anonymous visitor id can't claim. The binary migrates and serves its own SQLite database and serves the operations and the live socket under `/_leptoskit/services/analytics`.
- **`analytics_app`.** A leptoskit app with `[services.analytics] package = "analytics_service"`. Its button calls `capture` from the browser, its dashboard watches `summary` live, and its page renders the first summary on the server by calling the service privately.

## 10. Scaling and pitfalls

**Replicas.** A service scales like the app on the same engine:

- **SQLite:** one process, because there's one writer and only commits through a process wake its live feed.
- **Postgres:** any number of replicas. Live queries across replicas need the relay. The relay names its stream after the database (`LEPTOSKIT_LIVE_<database>`, `rpc/server/relay/mod.rs`), so a database per service gives each service its own stream on the shared NATS server.

**Postgres connections.** Every process has its own pool (default 8; huddle uses 32), plus one direct connection per process for live's `LISTEN`. The total across the app and every service's replicas must fit the server's `max_connections` (100 by default on the image `lk infra` runs). `[services.<name>] pool` sets a service's pool size, and `lk doctor`, `lk infra plan`, and releases fail when the sum exceeds the limit (§8).

**A network hop.** A service call while the server renders adds a round trip that an app operation doesn't have. Pages should make independent service calls concurrently, and a slow service should make the page's request time out, not hang. Generated service clients get the same timeout and retry behaviour as the app's.

**Connections from browsers.** A screen watching live queries of the app and of two services holds three sockets. Over HTTP/2, which the edge speaks, that's cheap. The server-sent-events fallback over HTTP/1.1 counts against the browser's limit of six connections per origin, so in plain-HTTP dev with several services, prefer WebSockets.

**Rolling deploys.** The app and its services share a release, but a rollout isn't atomic, so for a while the new app can call an old service, or the reverse. Deploys roll out services before the app, and service operations change additively, as public app operations must already for native apps on older code-push releases.

**Cookies on public paths.** Public service paths share the app's origin, so browsers send the app's session cookie to the service, and a service response could set cookies for the app. That's acceptable for internal services, which the app's own team writes. For installed and remote services (`trust = "isolated"`, their default), the dev proxy and the generated Compose edge strip `Cookie` from requests and `Set-Cookie` from responses on service paths; Kubernetes and `lk infra` refuse isolated public services until their edge can strip them. `trust = "first-party"` lifts that for a service the app trusts. Service HTML and JavaScript served from the app's origin run with the app's privileges, so only first-party services should serve them.

**Installed services' build scripts.** Building an installed service runs its `build.rs`, as adding any dependency does. `services.lock` and `--locked` pin what runs.

**Readiness ordering.** In dev the app waits for every service to be ready. In deploys each process becomes ready on its own (Compose orders by `service_started`, not health), so an outage in analytics doesn't take the app's pages down. A page that needs a service should handle the service being unavailable.

**Names.** Service databases and roles live under `<app>_service_<name>`, apart from every name leptoskit uses itself (§3).

**Calling as the app.** Server rendering calls a service as the person it renders for when the app forwards identity to it, and anonymously otherwise, so its output equals what that person's browser could fetch. Privileged reads use `call_as_app` explicitly, and must stay on the server.

**Credentials on native apps.** A native app's bearer token reaches a service only when `service_credentials` names it, so an installed service never sees the app's tokens. What a service gets instead is the forwarded-identity token for it, when the app forwards identity.

**A token request per service.** In the browser and a native app, the first call to each service asks the app for a token first, and asks again every 5 minutes while the app has none to give. An app whose server doesn't serve `rpc.*` (no `rpc::server::Server`) answers 404, and its services' calls go anonymously.

## 11. Status

| Part                                                                         | Status and evidence                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      |
| ---------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `[services]` configuration, validation, manifest `name`, and schema          | built; `leptoskit_config` unit and integration tests                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                     |
| Service manifests; installed and remote sources, `services.lock`             | built; resolution tested against real throwaway workspaces; fetching from a real registry or git source isn't exercised                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                  |
| A database per service in dev                                                | built; SQLite run end to end in `lk dev` with `examples/analytics`; the Postgres path (database `<appdb>_service_<name>`) isn't run against a real server in `lk dev`, and on a branch database the service gets an empty database, not a copy                                                                                                                                                                                                                                                                                                                                                                           |
| `leptoskit_axum::service` runtime, `main!`, `expect_name`                    | built; unit tests and a real-port integration test                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                       |
| Service-owned operations, live queries, client routing                       | built for queries, mutations, and actions; `leptoskit_rpc` integration tests, and `examples/analytics` tests with unified features proving the app's server neither registers nor serves `capture` and routes calls to the service                                                                                                                                                                                                                                                                                                                                                                                       |
| `ServiceCallers` in both directions; the app's live socket refusing services | built; integration tests                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| `lk build`, `lk dev`, `lk serve`, `lk services`                              | built, with services started in a cleared environment, port checks before each start, and `SIGTERM` then `SIGKILL` on stop; proxy tests (routing, streaming, cookie stripping, WebSocket), and real `lk dev` and `lk build` + `lk serve` runs of `examples/analytics` (captures through the proxy, server-rendered counts, a 101 upgrade to the service's live socket)                                                                                                                                                                                                                                                   |
| `examples/analytics`                                                         | built; a click in a real browser updating the count live isn't verified (headless Chrome on the dev machine wouldn't load loopback URLs); the same paths are covered by curl and tests                                                                                                                                                                                                                                                                                                                                                                                                                                   |
| Deploy generators, `lk infra`, connection budget (§8)                        | built; insta snapshots, the per-service roles, setup idempotence, and Compose init run against real Postgres, and recovery with restic; Kamal, Compose, Caddy, and Kubernetes outputs aren't run for real                                                                                                                                                                                                                                                                                                                                                                                                                |
| `lk db --service`                                                            | not started; regenerate a service's migrations with a temporary `leptoskit.toml`                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                         |
| `--rollback-schema` and release history for service databases                | not started; app database only                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                           |
| Service-owned jobs, cron, streams, and channels                              | built; `tests/services.rs` shows the app never schedules, enqueues, or runs a service's job and doesn't find its streams or channels, while the service's process does                                                                                                                                                                                                                                                                                                                                                                                                                                                   |
| Forwarding the signed-in person's identity to a service                      | built; unit tests for the key, minting, tampering, audience, expiry and skew; `tests/services.rs` over real HTTP and sockets (mint through `rpc.service_token`, the service seeing `person:<id>`, another service and tampered or expired tokens refused, no token without `forward_identity`, server rendering signing as the person rendered for, a live socket renewing then refusing at expiry); the client's `IdentityToken` against a scripted app; `examples/analytics`'s `mine` over HTTP and live. The example app has no sign-in, so a browser forwarding a real `leptoskit_auth` session isn't run end to end |
| Several application hosts with `lk infra`                                    | not started; refused while services exist                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                |

## 12. Open questions

- **Several hosts under Kamal.** Network aliases resolve on one Docker host (§8). Spreading services across hosts needs a private address per host, such as kamal-proxy on each host or the private network's IPs.
- **Edge cookie stripping on Cloudflare.** A transform ruleset is the zone's single entry point; owning it safely needs merging with existing rules.
- **Identity across services.** Forwarding (§5) gives a service the person's `Actor` UUID and nothing else. Open: whether apps need to forward more (a tenant, roles) as signed claims; whether a revoked session should end a service's view sooner than the token's 5 minutes, which needs the service to ask the app or the app to push revocations; and a service calling the app _as_ the person, which forwarding deliberately doesn't allow.
