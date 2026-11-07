# Services

Some of your backend shouldn't share a process with the server that renders your pages. Product analytics is the usual example: every page view and click becomes a write, the dashboards run heavy queries, and a burst of events, or a slow report, shouldn't slow down rendering or take the site down with it. The analytics code also has no business holding your app's database credentials.

A **service** is a separate Rust program your app declares in its `leptoskit.toml`. It runs beside the app's server, on the same machine or another one, with:

- **its own process and port**, which `leptoskit dev` and `leptoskit serve` start for you;
- **its own data**, in a database of its own on the same engine as the app's;
- **its own operations**: queries, mutations, and actions, including live queries over its own data.

Your screens and your server call a service's operations exactly as they call the app's own: with `call`, `use_rpc_query`, `use_live`, and `use_mutation`. leptoskit sends each call to the right process.

Services are server-side. The screens that use them run everywhere: the web, iOS, Android, macOS, Windows, Linux, the terminal, and HarmonyOS (experimental). In the browser, a public service answers on your app's own origin, under `/_leptoskit/services/<name>/`, so there are no cross-origin requests. Native apps reach it through the origin they pass to `client::configure`, the same as the app's server.

The complete example in this guide is in [`examples/analytics`](https://github.com/leptoskit/leptoskit/tree/main/examples/analytics): a page that records events from the browser and a dashboard that watches the counts live.

## Declare the service

A service is a crate. The simplest kind lives in your app's Cargo workspace, next to the app's other crates:

```text
my_app/
├── leptoskit.toml
├── my_app/                  the app: screens and server
└── analytics_service/       the service: its operations, its database, and its binary
```

The app names it in `leptoskit.toml`:

```toml
[services.analytics]
package = "analytics_service"
```

The service describes the rest itself, in its own `Cargo.toml`:

```toml
[package.metadata.leptoskit.service]
name = "analytics"
port = 3100
public = true
database = true
features = ["service"]
```

`name` is required, and it must be the name your `[services.<name>]` table uses. It's the name the service's operations are declared with, and every call is addressed to it, so `leptoskit` stops with an error that names both when the table is called something else.

That's all `leptoskit dev` needs: it builds the service, gives it a database, starts it on port 3100, waits until it's ready, and then starts the app.

A service doesn't have to live in your workspace. The same table installs one from elsewhere, or connects to one that's already running:

```toml
# Built from a registry, git, or a directory outside the workspace.
[services.search]
crate = "acme-search-service"
version = "0.4"
# git = "https://github.com/acme/search", rev = "…"
# path = "../search"

# Already running somewhere else: leptoskit runs nothing and only connects the app to it.
[services.billing]
url = "https://billing.internal.example"
```

Each table has exactly one source: `package`, `path`, `crate` (with an optional `version`), `git` (with at most one of `rev`, `branch`, or `tag`), or `url`. A service's name starts with a lowercase letter and holds lowercase letters, digits, and `-`. `volume` and names starting `volume-` are reserved.

Building an installed service resolves its exact version once and records it in `services.lock` at the root of your project. Commit that file: release builds use it with `--locked`, so the same commit always builds the same service, and they fail if it's missing.

A remote service takes only `url`, `public`, `routes`, `bind`, `trust`, and `forward_identity`. Its token comes from your environment as `LEPTOSKIT_SERVICE_<NAME>_TOKEN`, such as `LEPTOSKIT_SERVICE_BILLING_TOKEN`.

### Settings

The service's `[package.metadata.leptoskit.service]` sets its defaults, and `[services.<name>]` in your `leptoskit.toml` can override any of them. A value in `leptoskit.toml` replaces the service's, and `env` and `bind` merge key by key, with `leptoskit.toml` winning.

| Key                | Meaning                                                                                                                                                                                                 |
| ------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `bin`              | Which binary to run, when the package has several.                                                                                                                                                      |
| `features`         | Cargo features to turn on when building the service.                                                                                                                                                    |
| `port`             | Where it listens. Required for every service that runs as a process. Unique, and not `serve.port`.                                                                                                      |
| `public`           | Whether browsers and native apps can reach it at `/_leptoskit/services/<name>/` on the app's origin. Off by default: a private service only answers the app's server.                                   |
| `routes`           | Extra public path prefixes, such as `/_analytics`. Needs `public`. Can't be `/`, overlap another service's, or cover the app's own `/pkg`, `/api`, `/live`, or `/_leptoskit`.                           |
| `database`         | Whether the service gets a database of its own: on the engine your `[db]` names, or SQLite when the app has no `[db]`.                                                                                  |
| `pool`             | How many connections the service's database pool should hold.                                                                                                                                           |
| `env`              | Plain values set on the service.                                                                                                                                                                        |
| `secrets`          | Names of variables the service needs from your environment.                                                                                                                                             |
| `volumes`          | Named directories the service keeps between runs.                                                                                                                                                       |
| `bind`             | Variables set on the **app** when this service is present. `{url}`, `{token}`, and `{public_path}` are filled in, so `ANALYTICS_PATH = "{public_path}"` gives the app `/_leptoskit/services/analytics`. |
| `trust`            | Only in `leptoskit.toml`: `"first-party"` or `"isolated"`. See [Trusting a service](#trusting-a-service).                                                                                               |
| `forward_identity` | Only in `leptoskit.toml`: whether the service learns who is signed in. On by default for `first-party` services, off for `isolated` ones. See [Knowing who is signed in](#knowing-who-is-signed-in).    |

## Write the service's operations

A service's operations live in its library, which the app depends on too. That's how the app's screens get the typed argument structs. Name the service on each operation:

```toml
# analytics_service/Cargo.toml
[dependencies]
leptoskit_db = "0.2"
leptoskit_rpc = { version = "0.2", features = ["client"] }
leptoskit_axum = { version = "0.2", optional = true }
schemars = "1"
serde = { version = "1", features = ["derive"] }

[features]
# The service's own build: the operations' bodies, its database, and its live socket.
service = [
	"dep:leptoskit_axum",
	"leptoskit_db/sqlite",
	"leptoskit_rpc/server",
	"leptoskit_rpc/sqlite",
	"leptoskit_rpc/live",
]

[[bin]]
name = "analytics_service"
required-features = ["service"]
```

The library depends on `leptoskit_db`, `leptoskit_rpc`, and `leptoskit_axum` directly rather than on `leptoskit`. When a crate depends on `leptoskit` at all, even optionally, `#[table]` expands against `leptoskit::db`, which only exists while `leptoskit`'s `db` feature is on. A service library that only turns its server side on under `service` should use the crates themselves, as above.

```rust
// analytics_service/src/lib.rs
use leptoskit_db::table;
use leptoskit_rpc as rpc;

/// The name `[services.analytics]` and the service's manifest use.
pub const SERVICE: &str = "analytics";

#[table(name = "events", live)]
pub struct Event {
	pub id: leptoskit_db::Id<Event>,
	pub name: String,
	pub distinct_id: String,
	#[db(created_at)]
	pub created_at: leptoskit_db::Timestamp,
}

/// Records one event. Anyone may call it: a browser sends its visitor id.
#[rpc::mutation(service = "analytics")]
pub async fn capture(cx: &rpc::MutationCx, event: String, distinct_id: String) -> rpc::Result<()> {
	NewEvent { name: event, distinct_id }.insert(cx.tx()).await?;
	Ok(())
}

/// Counts per event over the last `days` days: an aggregate, so anyone may
/// watch it, live.
#[rpc::query(service = "analytics")]
pub async fn summary(cx: &rpc::QueryCx, days: u32) -> rpc::Result<EventSummary> {
	// …
}
```

`service = "analytics"` does two things:

- **It keeps the body in the service.** The body only compiles when the library's own `service` feature is on, which is how the service binary builds it. In your app the operation is just its typed arguments, so the app doesn't compile the service's database code, and the app's server never serves the operation itself, even when a workspace-wide build turns `service` on everywhere.
- **It sends every call to the service.** In the browser and in native apps, calls go to `/_leptoskit/services/analytics/api/<operation>` on the app's origin. From the app's server, they go to the service's private address.

The feature must be called `service`.

Background jobs, cron schedules, streams, and channels belong to a service the same way:

```rust
/// Rolls up yesterday's events, every night.
#[rpc::cron("@daily")]
#[rpc::job(service = "analytics")]
pub async fn roll_up(cx: &rpc::JobCx) -> rpc::Result<()> {
	// …
}

/// Who's looking at the dashboard right now.
#[rpc::channel(message = Cursor, authorize = may_watch, service = "analytics")]
pub struct Dashboard {
	pub board: String,
}
```

- **Jobs and cron** run only on the service's workers. Workers take their service from the installed `Server` when they start, and building `rpc.router()` installs it, so start them after that, or name the service yourself with `Jobs::new(db).service(Some("analytics"))`. The service's workers then run its jobs and schedules, and the app's workers never do, even when a workspace-wide build links them into the app. Only the service enqueues its jobs: enqueueing one from the app is an error, so the app asks the service to by calling one of the service's operations.
- **Streams and channels** are served by the service's live socket. `use_stream` and `use_channel` open them over the service's connection, as `use_live` does for its queries.

## Write the service's binary

The binary connects to the service's database, migrates it, and serves the operations under the service's prefix:

```rust
// analytics_service/src/main.rs
use analytics_service::SERVICE;
use leptoskit_axum::axum::Router;
use leptoskit_axum::service::ServiceEnv;
use leptoskit_db::Db;
use leptoskit_rpc::server::Anonymous;
use leptoskit_rpc::server::Server;
use leptoskit_rpc::server::ServiceCallers;

static MIGRATIONS: &[leptoskit_db::Migration] = leptoskit_db::migrations!("migrations");

leptoskit_axum::service::main!(analytics_service, run);

async fn run(env: ServiceEnv) -> Result<Router, Box<dyn std::error::Error + Send + Sync>> {
	// Under any other name, the app's calls would go somewhere this isn't.
	env.expect_name(SERVICE)?;

	let db = Db::connect(env.database_url().ok_or("the service has no database")?).await?;
	db.migrate(MIGRATIONS).await?;

	let rpc = Server::new(db)
		.service(SERVICE)
		.identify(ServiceCallers::for_service(Anonymous));
	let live = match env.public_origin() {
		Some(origin) => rpc.live().origin(origin),
		None => rpc.live(),
	};
	live.start().await?;

	Ok(Router::new().nest(&env.public_prefix(), rpc.router().merge(live.router())))
}
```

`main!` writes `main` for you:

1. It reads what leptoskit tells the service (its name, address, token, database, pool size, and volumes) into a `ServiceEnv`.
2. It awaits your `run`.
3. It serves the router with `/healthz` and `/readyz` added, and drains in-flight requests on shutdown.

Its first argument is your library's name, which keeps the library's operations linked into the binary. A binary that never mentions its library would serve nothing.

`Server::new(db).service("analytics")` serves only the analytics operations. `ServiceCallers::for_service` identifies a request carrying the service's token as the caller `"app"`. Requests from browsers have no token, so they're identified by the `Identify` you wrap, here as anonymous. An operation only the app's server may call checks the caller, and the app's server calls it with [`call_as_app`](#calling-as-the-app):

```rust
/// The latest events, visitor ids and all. Only the app may call it.
#[rpc::query(service = "analytics")]
pub async fn recent(cx: &rpc::QueryCx, limit: u32) -> rpc::Result<Vec<RecentEvent>> {
	if cx.identity().caller.as_deref() != Some(rpc::server::APP_CALLER) {
		return Err(rpc::Error::forbidden("only the app reads individual events"));
	}
	// …
}
```

For a route that isn't an operation, put `env.require_token()` on it with `route_layer`, and only callers with the service's token get through.

## Call it from the app

The app depends on the service's library, without its `service` feature:

```toml
# my_app/Cargo.toml
[dependencies]
analytics_service = { path = "../analytics_service" }
leptoskit_rpc = { version = "0.2", features = ["client"] }

[features]
# The server calls the service over HTTP while it renders.
ssr = ["leptoskit_rpc/services"]
# The browser calls and watches it through the app's origin.
hydrate = ["leptoskit_rpc/web"]
```

A screen records an event and watches the count live:

```rust
use analytics_service::Capture;
use analytics_service::Summary;
use leptoskit::prelude::*;
use leptoskit_rpc::client::call;
use leptoskit_rpc::use_live;

#[component]
pub fn Signups() -> impl IntoView {
	let summary = use_live(|| Summary { days: 7 });
	let sign_up = move || {
		leptos::task::spawn_local(async move {
			let _ = call(Capture {
				event: "signup".into(),
				distinct_id: "visitor-1".into(),
			})
			.await;
		});
	};

	view! {
		<View class="flex-col gap-2 p-4">
			<Text>{move || format!("{} events this week", summary.data().map_or(0, |s| s.total))}</Text>
			<Button on_press=sign_up>"Sign up"</Button>
		</View>
	}
}
```

On every platform this screen runs on:

- **`capture`** goes straight to the service. On the web it's a `POST /_leptoskit/services/analytics/api/capture` on the page's own origin. Native apps send it to the origin they configured.
- **`use_live`** opens a second live connection, at `/_leptoskit/services/analytics/live`, beside the app's own. It's opened the first time a screen watches one of the service's queries. Updates arrive the way they do for the app's own [live queries](24-real-time.md): when a write to `events` commits, the service re-runs the counts it affects and sends what changed.
- **While the server renders the page,** the first count comes from the app's server calling the service over HTTP, at the address `leptoskit dev` gave it. The page arrives with the number already in it. The server calls as whoever the browser would: the signed-in person, when the service [knows who is signed in](#knowing-who-is-signed-in), and an anonymous visitor otherwise. So a page only ever shows what that person's browser could fetch for itself.

Server code calls a service with the same `call`:

```rust
let week = leptoskit_rpc::client::call(Summary { days: 7 }).await?;
```

That needs `leptoskit_rpc`'s `services` feature on the app's server (`rpc-services` on the `leptoskit` facade). Its `server` feature turns it on too. A service's operations can't run in the app's process, so each call is a network round trip. Make independent calls concurrently, and handle the error when a service isn't up: in a deployment, each process becomes ready on its own, so analytics being down doesn't take your pages down.

### Calling as the app

`call`, and every query hook (`use_rpc_query`, `use_live`, and the rest), reach a service as the browser would, as the signed-in person or anonymously, even on the server, which holds the service's token. That's deliberate: what the server renders goes into the page's HTML, so a hook that read an app-only operation there would hand its result to every visitor.

To call an operation only the app may call, such as `recent`, use `call_as_app` in server code:

```rust
let latest = leptoskit_rpc::client::call_as_app(Recent { limit: 20 }).await?;
```

It presents the service's token, so the service identifies the call as `"app"`. It exists only on the server, never in Wasm, and nothing calls it for you. Keep what it returns on the server, and check who's asking before you pass any of it on.

### Native apps and credentials

A native app signs its own calls with the credentials it configured, such as `leptoskit_auth`'s access token. Those don't go to services: a service gets only the short-lived token that [says who is signed in](#knowing-who-is-signed-in), when it should know, and a service refusing a request never renews the app's token. To trust a service with the app's own credentials instead, say so:

```rust
rpc::client::configure(
	ClientConfig::new()
		.origin("https://myapp.example")
		.credentials(app_credentials.clone())
		.service_credentials("analytics", app_credentials),
);
```

A service's live connection doesn't share the app's offline queue either: while it's down, the service's mutations go over HTTP.

## Its own database

With `database = true`, the service gets a database of its own on the engine your `[db]` table names, or SQLite when your app has no `[db]`:

| Your app uses        | The service gets in `leptoskit dev`                                   |
| -------------------- | --------------------------------------------------------------------- |
| SQLite, or no `[db]` | the file `.leptoskit/services/<name>/<name>.sqlite3`                  |
| Postgres             | the database `<your database>_<name>`, on the same development server |

The service finds it in `DATABASE_URL`. It applies its own migrations when it starts, as in the binary above, so an installed service brings its tables with it. Generating a service's migrations from its `#[table]` structs isn't part of `leptoskit db` yet; write them in the service's `migrations/` directory.

The service's data is its own: the app can't query the service's tables, and the service can't query the app's. That separation is what lets each one run its own live queries and background work without the other interfering.

### Reading the app's data

A service reads and writes the app's data only through the app's ordinary operations, never through its database or its live queries:

- **Generate a client.** Generate the app's Rust client with [`leptoskit rpc generate`](13-sharing-the-rpc.md), and have the service depend on it. Don't depend on the app's operations crate, which would compile the app's operations into the service.
- **Call the app.** Send requests to `LEPTOSKIT_APP_URL` with `Authorization: Bearer` and the service's own `LEPTOSKIT_SERVICE_TOKEN`.
- **Identify services on the app.** On the app's server, wrap its `Identify` in `ServiceCallers::for_app(…)`. A service's calls then arrive as the caller `service:<name>`, such as `service:analytics`, and your operations and policies decide what that caller may see.

The app's live socket refuses a service with `403`. A service that needs fresh app data asks for it, or the app tells it by calling one of the service's operations.

## Trusting a service

Public service paths share your app's origin, so browsers send them the app's cookies, including the session cookie. That's fine for a service your own team writes in your workspace. It isn't for code you installed from elsewhere.

`trust` decides what happens:

- **`"first-party"`** is the default for `package` services. Requests reach the service as they are.
- **`"isolated"`** is the default for installed and remote services. `leptoskit dev` and `leptoskit serve` remove the `Cookie` header from requests to the service and `Set-Cookie` from its responses, so it never sees the app's session and can't set cookies for your origin.

Set `trust = "first-party"` on an installed service only if you'd trust its code with your users' sessions. A service serving HTML or JavaScript from your origin runs with your app's privileges, so only first-party services should.

## Knowing who is signed in

An analytics dashboard that shows each person their own activity needs the service to know who is asking. But the service never sees your app's session: an isolated service gets no cookies, and a native app's access token stays with the app. Instead, your app vouches for the person, one service at a time.

A service you write in your workspace is told by default. For an installed or remote one, turn it on in its table:

```toml
[services.search]
crate = "acme-search-service"
forward_identity = true
```

The service's operations then see the person. Here is the analytics example's `mine`, which returns the signed-in person's own events:

```rust
/// The signed-in person's own latest events, newest first.
#[rpc::query(service = "analytics")]
pub async fn mine(cx: &rpc::QueryCx, limit: u32) -> rpc::Result<Vec<RecentEvent>> {
	let Some(me) = cx.identity().context.get::<rpc::server::ForwardedIdentity>() else {
		return Err(rpc::Error::forbidden("sign in to see your own events"));
	};
	// `me.person` is the person's id: their account's, with `leptoskit_auth`.
	// …
}
```

A screen watches it like any other live query, on every platform, and gets only that person's events:

```rust
let mine = use_live(|| Mine { limit: 20 });
```

Who the service sees, depending on where the call comes from:

- **In the browser and in native apps,** before the first call to the service, the client asks your app's server for a token for that service, signed in the way the app already is (its cookie, or a native app's access token). The token names the person and lasts five minutes. The client sends it with that service's calls and on its live connection, and nowhere else, and fetches a new one before it runs out.
- **While the server renders a page,** the server signs the service's calls with a token for the person it's rendering for, so the page holds what that person's browser would fetch.
- **When nobody is signed in,** calls go anonymously, as before.

The operation sees the caller `person:<id>`, apart from the app's own `"app"`, and the person's `Actor`, so `#[db(created_by)]` columns and policies on the service's own tables work as they do in the app.

What the service learns, and what it can't do with it:

- It learns the person's id and nothing else: not their session, email, or roles.
- A token for one service doesn't work at any other service, or at your app.
- When someone signs out, `leptoskit_auth`'s client forgets their tokens and the client stops asking for more. A token already handed out keeps working until it expires, at most five minutes later. If you don't use `leptoskit_auth`'s client, call `rpc::client::reauthenticate()` after signing in or out.

It needs two things you probably already have: your app's server serves `rpc::server::Server` with your sign-in as its `Identify`, which answers the client's request for a token, and the service builds its server with `ServiceCallers::for_service`, which accepts tokens made for it.

Without `forward_identity`, services see browsers as anonymous, and the app's server as `"app"` when it uses `call_as_app`.

## Running it

- **`leptoskit dev`** builds the app and its workspace services in one go, starts each service and waits for it to be ready, then starts the app. Output from each service is labeled with its name. When a service is public, `leptoskit dev` listens on `serve.port` itself: it sends each public service's paths to that service and everything else to your app. Responses still stream, and WebSockets and server-sent events pass through. A change rebuilds and restarts the services and the app together.
- **`leptoskit serve`** does the same with what the last `leptoskit build` produced, in production mode.
- **`leptoskit services`** lists what the app declares and how each service resolved: its source, port, trust, database, public paths, volumes, and the variables it binds. `--json` prints the same as JSON. It never prints tokens or passwords.

A remote service that's public isn't proxied by `leptoskit dev`; it prints a warning instead.

### What each process receives

A service receives:

| Variable                            | Value                                                                                  |
| ----------------------------------- | -------------------------------------------------------------------------------------- |
| `LEPTOSKIT_SERVICE_NAME`            | its name                                                                               |
| `LEPTOSKIT_SERVICE_ADDR`            | the address to listen on                                                               |
| `LEPTOSKIT_SERVICE_TOKEN`           | its token: callers that present it are trusted, and the service presents it to the app |
| `LEPTOSKIT_PUBLIC_ORIGIN`           | the app's public origin                                                                |
| `LEPTOSKIT_APP_URL`                 | the app server's private address                                                       |
| `DATABASE_URL`                      | its own database, with `database = true`                                               |
| `LEPTOSKIT_SERVICE_DATABASE_POOL`   | its database pool size, when `pool` is set                                             |
| `LEPTOSKIT_SERVICE_VOLUME_<VOLUME>` | each volume's directory                                                                |

plus its `env` values. `ServiceEnv` reads all of these for you.

The app receives, for each service:

| Variable                            | Value                                                  |
| ----------------------------------- | ------------------------------------------------------ |
| `LEPTOSKIT_SERVICE_<NAME>_URL`      | the service's private address                          |
| `LEPTOSKIT_SERVICE_<NAME>_TOKEN`    | the service's token                                    |
| `LEPTOSKIT_SERVICE_<NAME>_IDENTITY` | `forward`, when the service knows who is signed in     |
| each `bind` entry                   | with `{url}`, `{token}`, and `{public_path}` filled in |

`<NAME>` is the service's name in capitals, with `-` written `_`.

## Deploying

A service ships in the same image and the same release as your app. `leptoskit build` builds each service's binary, and the image `leptoskit generate deploy` writes copies it to `/app/services/<name>`. The image still starts your app by default; each service runs the same image with its own command. Rolling back restores the app and its services together.

What each deploy target makes of a service:

- **Compose** (`--target compose`): an entry per service running `/app/services/<name>`, with its variables, its volumes, and a health check, which the app waits for. Public services get a small Caddy edge in front of the app that sends their paths to them, strips cookies for `isolated` services, and passes WebSockets through. It needs Docker Compose 2.23.1 or newer. On Postgres, each service gets its own role and database the first time the database volume starts; on SQLite, each service gets its own data volume.
- **Kubernetes** (`--target kubernetes`, with or without `--split-roles`): a Deployment, a Service, and a Secret per service, checked on `/readyz` and `/healthz`. A service with a SQLite database or volumes runs one replica. An Ingress sends each public service's paths to it ahead of the app's catch-all. Stripping cookies for `isolated` services is left to your ingress controller, and the generated next steps say so.
- **`leptoskit infra`**: each service runs as its own role on the primary host, with a stable network name the app calls, and kamal-proxy sends public paths to it. Every release creates or updates each service's Postgres role and database, and backups and restores cover every service's database as well as the app's. Services need a single application host for now (`instances = 1`), and `leptoskit infra plan` refuses public `isolated` and public remote services, because the edge can't strip their cookies yet.
- **Fly and systemd** don't support services yet: generating them stops with an error that names the targets that do.

Every process keeps its own Postgres connections, so `leptoskit infra plan`, releases, and `leptoskit doctor` add up the app's and each service's pools and stop when they don't fit the server's limit. `[infra.postgres]` sets that limit (`max_connections`, 100 by default) and the app's pool size (`app_pool`, 8 by default); a service's comes from its `pool`.

Services start before the app, so a new app never calls a service that's still on the previous release. Change a service's operations only by adding to them, as you would for native apps still on an older release.
