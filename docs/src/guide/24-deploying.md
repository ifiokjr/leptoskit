# Deploying a server

A server-rendered app works on your machine with `leptoskit serve`, but a host somewhere else needs more than the binary. It needs a container that builds the same way every time, the `LEPTOS_*` environment `leptoskit serve` sets for you, somewhere to keep the database, and a way to tell the load balancer when an instance can take traffic and when it's about to stop. Get the last part wrong and every deploy drops the requests in flight.

`leptoskit generate deploy` writes those files for you, from `leptoskit.toml` and `Cargo.toml`:

```bash
leptoskit generate deploy --target fly
```

```text
created Dockerfile
created .dockerignore
created fly.toml

next:
  1. Create the app from fly.toml: `fly launch --copy-config --no-deploy`
  2. Create the database's volume: `fly volumes create data --size 1`
  3. Deploy: `fly deploy`
```

This page covers what the files contain, the health checks and graceful shutdown they rely on, and what each target gives you. It's about the app's server, the binary that renders pages and answers server functions in `ssr` and `islands` apps. Your web, iOS, Android, macOS, Windows, Linux, terminal, and HarmonyOS clients talk to that server wherever it runs; nothing about them changes. A `csr` app has no server to deploy: upload `dist/` to a CDN or bucket instead (see [security and scale](16-security-and-scale.md#client-rendered-apps-the-cdn-is-the-server)).

## The targets

| Target    | What it writes                | What you get                                                                                                                                          |
| --------- | ----------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------- |
| `docker`  | `Dockerfile`, `.dockerignore` | An image for any container platform.                                                                                                                  |
| `compose` | the above, and `compose.yaml` | The app and its database on one machine with `docker compose up`: a Postgres service with its own volume, or a volume for the SQLite file.            |
| `fly`     | the above, and `fly.toml`     | An app on Fly.io whose proxy checks `/readyz`, gives the server time to stop, and keeps machines running. SQLite gets a volume; Postgres is attached. |

The files are yours. Edit them as you like: running the generator again changes nothing, says so, and keeps any file you've edited. Pass `--force` to replace edited files with the current templates. Each target writes the same `Dockerfile`, so you can generate `compose` for a staging machine and `fly` for production in the same project.

The generator reads:

- `app.name`, the server package, and its binary: the one named after the package, or its only one;
- `serve.port`, the port the container listens on;
- `build.client_package`, whose library name the site's `pkg/` files are named after;
- `[db]`: the dialect, and `url_env`, the variable that holds the database's URL;
- `[auth]`: the variables that hold its keys and mail settings, which become secrets.

It tells you when something is missing, such as a `[[bin]]` it can't choose between, or an `app.name` that isn't a package in the workspace.

## The container

The `Dockerfile` has two stages:

1. **Build.** It installs the leptoskit CLI at the version your project uses and runs `leptoskit build --release`, the same build you run on your machine, so the server, the wasm client, and the precompressed site come out exactly as they do there. A `rust-toolchain.toml` at the root picks the Rust release; otherwise the image uses the latest stable.
2. **Run.** A small Debian image with the server and `dist/`, nothing else. It sets the `LEPTOS_*` environment (`LEPTOS_SITE_ADDR=0.0.0.0:3000`, `LEPTOS_SITE_ROOT=/app/dist`, `LEPTOS_OUTPUT_NAME`, and `LEPTOS_ENV=PROD`), runs as an unprivileged user, and writes nothing to its own filesystem, so it runs with a read-only root filesystem. A SQLite database lives on a volume at `/data`, the one path it writes. A `HEALTHCHECK` calls `/healthz`.

Secrets never go in the `Dockerfile`, `compose.yaml`, or `fly.toml`. The Postgres URL, `AUTH_ENCRYPTION_KEYS`, and mail settings come from the environment when the container starts: `docker run -e`, an `.env` file beside `compose.yaml`, or `fly secrets set`. The `.dockerignore` keeps `.env` files, local databases, and build output out of the image.

To build and run the image on your machine:

```bash
docker build -t my_app .
docker run --rm --read-only --tmpfs /tmp -p 127.0.0.1:3000:3000 my_app
```

## Health checks

A load balancer asks every instance two questions, and the server answers each on its own route:

- **`/healthz`: is the process up?** It answers `200 ok` for as long as the server runs. Docker uses it to restart a container that has hung.
- **`/readyz`: should it get traffic?** It answers `200` only when every readiness check passes, and `503` otherwise, with the checks that fail:

```json
{ "failing": ["migrations"], "ready": false }
```

A readiness check starts failing, and your server passes it when the work it stands for is done. A new project's server already has both routes and a `migrations` check:

```rust
use leptoskit::server::GracefulShutdown;
use leptoskit::server::Readiness;

let readiness = Readiness::new();
let migrated = readiness.check("migrations");

for name in db.migrate(my_app_db::MIGRATIONS).await? {
	println!("applied migration {name}");
}

migrated.pass();

let app = my_app_backend::router(db)
	// ...the site's routes...
	.with_state(options)
	.merge(readiness.routes());
```

`Readiness` is cheap to clone, and every clone shares the same checks, so a task that owns a check, such as one that keeps a connection open, can take its own copy and call `fail()` and `pass()` as things change.

A project created before this release doesn't have the routes. The generator notices, and its next steps tell you to add them, since the `HEALTHCHECK` and Fly's checks depend on them.

## Stopping without dropping requests

Platforms replace an instance by sending it `SIGTERM`, then killing it if it hasn't exited after a timeout. In that window the server has to stop getting new traffic and finish what it has. `serve_listener_with` does it in order:

1. `/readyz` starts answering `503`, so the load balancer takes the instance out of rotation.
2. The server keeps serving for the **drain delay**, 5 seconds by default, because load balancers only probe every few seconds and still send traffic until they notice.
3. It stops accepting connections and finishes the requests in flight within the **grace period**, 20 seconds by default.
4. If requests are still running after that, `serve_listener_with` stops waiting and returns an error that says so. When `main` returns it, the process exits and they're cut off.

```rust
let shutdown = GracefulShutdown::new().readiness(&readiness);
leptoskit::server::serve_listener_with(listener, app, shutdown).await?;
```

With the defaults the server is gone within 25 seconds, and the generated files give it 30: `stop_grace_period: 30s` in `compose.yaml`, and `kill_timeout = "30s"` in `fly.toml`. If you change the timings, change those too:

```rust
use std::time::Duration;

let shutdown = GracefulShutdown::new()
	.readiness(&readiness)
	.drain_delay(Duration::from_secs(10))
	.grace_period(Duration::from_secs(45));
```

`serve_listener` and `serve_router` still work as they always have: they stop accepting at once on `SIGTERM` or Ctrl-C and wait for every request, with no readiness and no limit.

## Docker Compose

`compose.yaml` runs the image with a read-only filesystem, a `/tmp` in memory, and the server's port reachable from the same machine only, for a proxy that terminates TLS in front of it.

- **With Postgres** (`[db] dialect = "postgres"`), it adds a `db` service running Postgres 17 with its data on a volume. The app waits for the database's own health check before it starts, and its `DATABASE_URL` is built from `POSTGRES_PASSWORD`, which you set in your shell or in `.env`.
- **With SQLite**, the database file lives on a `data` volume, so it survives `docker compose down` and rebuilds.

```bash
export POSTGRES_PASSWORD="$(openssl rand -hex 24)"
docker compose up --build
```

Compose refuses to start when a secret it needs isn't set, and names it.

## Fly.io

`fly.toml` builds the `Dockerfile`, and sets:

- **checks on `/readyz`**, so Fly's proxy sends requests only to machines that can serve them, and gives a new machine 30 seconds to run its migrations before the checks count;
- **`kill_timeout = "30s"`**, past the server's drain delay and grace period;
- **`auto_stop_machines = "off"` and `min_machines_running = 1`.** Fly can stop machines it thinks are idle, and a stopped machine drops every WebSocket and streaming connection on it, so the clients of an app that holds them would disconnect without warning. If your app only answers short requests, `"stop"` with `min_machines_running = 0` lets it scale to zero and start again on the next request.

For the database:

- **SQLite** goes on a volume mounted at `/data`. A volume belongs to one machine, so the app runs on one; that's enough for thousands of users. Create the volume before the first deploy with `fly volumes create data --size 1`.
- **Postgres** comes from `fly postgres create` and `fly postgres attach <name>`, which stores the URL as the `DATABASE_URL` secret. Any Postgres works the same way: set `DATABASE_URL` yourself with `fly secrets set`.

```bash
fly launch --copy-config --no-deploy
fly secrets set AUTH_ENCRYPTION_KEYS="$(date +%Y-%m):$(openssl rand 32 | basenc --base64url | tr -d =)"
fly deploy
```

## Limits

- The image builds from the project's root. Crates outside it, such as a local leptoskit checkout passed with `--sdk-path`, aren't in the build context, so the build can't find them.
- The build copies the server from `target/release`. If `.cargo/config.toml` moves the target directory, change the `cp` line in the `Dockerfile` to match.
- The first build compiles the leptoskit CLI and every dependency, which takes several minutes. Later builds reuse Docker's caches.
- Kubernetes and systemd targets aren't there yet. The `docker` target's image works on Kubernetes today: point a readiness probe at `/readyz`, a liveness probe at `/healthz`, and set `terminationGracePeriodSeconds: 30`.
