# Deploying a server

## Managed deployment

To deploy the website, API, and live connections together, `lk infra` provisions Linux hosts behind a Cloudflare tunnel, a public R2 bucket for signed code-push artifacts, and a separate private backup bucket. Kamal runs the application and Postgres containers. Start with one Hetzner x86 VM; DigitalOcean and multiple application hosts are also supported. This managed preset requires Postgres. The deployment exporters below still support SQLite.

Web, iOS, Android, macOS, Windows, Linux, terminal, and HarmonyOS (experimental) clients can reach this backend. Managed code-push publication targets iOS and Android; other generated native hosts do not yet install code-push releases. Store signing and host-binary distribution remain separate.

The parts fit together like this:

```text
leptoskit.toml + Rust app + migrations + Dockerfile
         |                    |
         | lk infra           | build and push a unique image tag
         v                    v
    OpenTofu providers     lk deploy -> Kamal
         |                    |
         v                    v
  VM(s), private network,  website + API + live services
  tunnel, DNS, buckets     Postgres on the primary host
         ^                    |
         |                    v
  Cloudflare <- clients   encrypted Restic -> private R2/B2
```

### Configure and provision

Add this to an existing app with `[db] dialect = "postgres"`. Replace the account IDs, SSH key ID, administrator CIDR, bucket names, image repository, and domains:

```toml
[infra]
name = "notes-production"
domain = "app.example.com"
compute = "hetzner"
location = "hel1"
server_type = "cx23"
instances = 1
ssh_key_ids = [123]
ssh_cidrs = ["192.0.2.1/32"]
account_id = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
zone_id = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
updates_domain = "updates.example.com"
updates_bucket = "notes-releases"
backup_bucket = "notes-backups"
image = "ghcr.io/team/notes"
registry = "ghcr.io"
registry_username = "team"
secrets = []

[infra.backups]
enabled = true
schedule = "daily"
daily = 7
weekly = 4
monthly = 3
max_age_hours = 36
```

Install OpenTofu 1.10 or later (before 2.0), Kamal 2, and SSH on the deployment machine. Supply `HCLOUD_TOKEN` and `CLOUDFLARE_API_TOKEN` through its environment and keep the authorized SSH key in its agent. The Cloudflare zone and provider SSH key must already exist. Verify each new server's SSH host-key fingerprint through a trusted provider console and add its key to `known_hosts` before the first SSH operation. Host-key verification stays enabled. Credentials never belong in TOML.

```sh
lk infra generate
lk infra plan
lk infra apply
lk infra output
```

`lk tofu` aliases `lk infra`. Generation writes inspectable `infra/notes-production/main.tf.json` and reuses the Dockerfile generator. The `.tf` and `.tf.json` formats originated in Terraform and are supported by OpenTofu. OpenTofu runs on your machine or CI runner and calls provider APIs; you do not need to host an OpenTofu server. Its state maps configuration to the resources those APIs created.

Commit the TOML, generated infrastructure configuration, migrations, Dockerfile, and `.terraform.lock.hcl`. Keep state, secret-bearing plans, and runtime files private. Changed generated configuration needs review and `lk infra generate --force`. Apply accepts only the saved plan whose configuration, provider lock, and plan fingerprints still match. Failed or interrupted applies consume that plan: plan again before retrying. Data-bearing primary hosts, networks, and buckets have destroy protection. LeptosKit also inspects the saved plan and blocks their deletion or replacement when a resource block is removed, including during a provider change. Stateless app replicas can still be scaled down.

Local state is the default. For CI or several operators, bootstrap an independent AWS S3 bucket with versioning and scoped access, then add:

```toml
[infra.state]
bucket = "independent-infrastructure-state"
key = "notes/production.tfstate"
region = "eu-west-1"
```

This enables server-side encryption and native state locking using the operator's AWS credentials. The preset does not bootstrap this bucket or enable OpenTofu client-side state encryption. Changing an existing backend requires an explicit OpenTofu state migration; never apply empty state against existing resources. Keep a recovery copy outside the application VM. Release history is shared on the primary host and included in encrypted backups.

### Choose a provider and check cost

For DigitalOcean, set `compute = "digitalocean"`, a location such as `ams3`, a size such as `s-1vcpu-2gb`, existing DigitalOcean SSH key IDs, and `DIGITALOCEAN_TOKEN`. Application replicas share the primary's private network. Provider changes do not migrate existing disks or database contents; plan a backed-up replacement environment explicitly.

To provision the private backup bucket in Backblaze instead of R2, add:

```toml
[infra.b2]
region = "us-west-004"
```

Provisioning uses `B2_APPLICATION_KEY_ID` and `B2_APPLICATION_KEY` for a B2 account in that region. The backup worker gets a separate bucket-scoped S3 key. Signed code-push artifacts stay on R2 behind the updates hostname.

`lk infra offers --min-memory-gb 2` queries the providers whose tokens are present and lists current x86 sizes and locations, grouped by currency and sorted by monthly base price. Hetzner quotes include primary IPv4; DigitalOcean quotes use the size's monthly price. It does not exchange currencies or select a paid replacement automatically. Compare regions, memory, transfer allowances, and your workload before selecting a size. Building images off the production host and leaving cache/relay disabled keeps the small preset lean.

An optional guard fetches current prices again before both plan and apply:

```toml
[infra.budget]
currency = "EUR"
monthly_compute = 10.0
```

This ceiling covers compute plus primary IPv4 for all configured hosts. It excludes tax, domains, backup/object storage, excess traffic, and other services, so it is not a total billing cap. A second host doubles the compute component. Capacity and account quotas can still prevent provisioning.

### Release the app

Build and push the image on a build machine. Before building, configure production auth origins, HTTP/WebSocket origin allowlists, and client API endpoints for the public hostname. Set distinct `POSTGRES_PASSWORD` (cluster administrator) and `DATABASE_PASSWORD` (application database owner), plus `KAMAL_REGISTRY_PASSWORD`, configured auth/mail secrets, and variables named by `infra.secrets` in the deployment environment. The app owns its database without superuser privileges. These password variables initialize a new data directory; changing them does not rotate an existing role's password.

Backups are enabled by default. Before first setup, also supply these independent secrets and preserve the encryption password outside the VM:

- `LEPTOSKIT_BACKUP_PASSWORD`: Restic repository encryption password;
- `LEPTOSKIT_BACKUP_ACCESS_KEY_ID` and `LEPTOSKIT_BACKUP_SECRET_ACCESS_KEY`: S3 credentials scoped to the private backup bucket.

```sh
lk infra build --version release-001 --push
lk deploy --setup --version release-001
# Build and push another unique tag with `lk infra build`:
lk deploy --version release-002
lk infra releases
lk infra status
lk deploy --rollback release-001
```

`--setup` waits for cloud-init, prepares the hosts, starts missing accessories, installs the backup schedule, and takes an initial backup with a restore check. Later releases leave accessories alone. The app must expose `/readyz` and `/healthz`. Kamal waits for readiness, replaces one host at a time, and drains old connections for 30 seconds. Live clients reconnect and resume; an existing WebSocket cannot move between processes.

Use `lk infra build --version <tag> --push` to attach migration metadata to the image. Deployment pulls and checks this metadata against the intended schema before stopping writers or starting the application. A label is a build consistency check, not an attestation of untrusted code; use a trusted build pipeline and protect registry access. Registry login uses existing Docker credentials or the configured registry password in a temporary credential directory.

Each successful release records the actual registry digest and database migration history on the primary server. Rollback/recovery pulls and checks the recorded digest before changing traffic, then boots the checked local image. Ordinary pushes refuse an existing registry tag. CI uses `--reuse-existing` to pull and check an already published artifact on retries. This checks migration metadata; callers must still choose a tag tied to the intended source revision. Protect registry writes and serialize publishers, since a registry without immutable-tag enforcement cannot make concurrent check-and-push atomic. Retain old registry images for rollback. Losing one operator workspace does not lose the shared release history.

Commands that change the running environment share a primary-host reservation, including scheduled backups. Losing a runner leaves its reservation in place. `lk infra status` shows its UUID and last release phase. Confirm that the old runner has stopped and inspect the environment before explicitly using `lk infra unlock --owner <uuid> --yes`. There is no automatic lock expiry.

### Change or roll back the schema

A changed migration history requires `lk deploy --maintenance --version <new-tag>`. This stops app writers, takes a verified backup when backups are enabled, and starts the replacement, which applies embedded migrations at startup. Review both SQL directions first. This conservative path has downtime and can leave the app stopped on failure. Stop external database writers too.

An image-only rollback requires the target's schema to match the actual database. For a schema-changing rollback, keep the newer migration files in the checkout and explicitly run:

```sh
lk deploy --rollback release-001 --rollback-schema --yes
```

This checks the recorded image, stops all app hosts, backs up the database, verifies there are no remaining database clients, and runs the reviewed `down.sql` files newest first. Each migration is transactional. Missing reverses, checksum mismatches, and unresolved data-recovery stubs block reversal. Successful reversals remain committed if a later reverse fails; the app stays stopped for repair. [Database rollback](22-database.md#rolling-back-a-migration) explains data-loss limits and how to review generated reverses. A down migration cannot reconstruct deleted user data without a deliberate recovery procedure.

On failed deployment, LeptosKit attempts the previous recorded image only if the actual schema still matches it. It does not automatically reverse migrations. After repairing or restoring data, `lk deploy --recover --version <recorded-tag>` starts a compatible recorded image. A mismatch discovered after deployment stops app containers and leaves shared history unchanged.

### Back up, verify, and restore

The primary runs a systemd timer for encrypted Restic backups. Each backup captures a consistent Postgres archive and release history, uploads them, restores the archive into an isolated temporary Postgres container, and compares migration history. Retention runs only after that restore check succeeds. Defaults retain seven daily, four weekly, and three monthly snapshots. Backups older than 36 hours or failed verification/retention make status unhealthy.

```sh
lk infra backups install  # update credentials, schedule, or retention
lk infra backups run
lk infra backups list
lk infra backups verify --snapshot latest
lk infra backups status
lk infra restore --snapshot <snapshot-id> --yes
lk infra releases
lk deploy --recover --version <compatible-recorded-tag>
```

Restore verifies the selected archive, stops app writers on every host, restores into a new database, and verifies its migration history before replacing the active database name. The previous database remains on disk under the name printed in the result. Restore also clears the disposable cache and starts a fresh relay stream so events cannot be replayed from ahead of the restored database. Previous relay files are retained beside the data directory. Incomplete relay/cache recovery blocks application releases until restore is retried successfully. The app stays stopped until explicit recovery. Restore requires free disk and memory for both databases and a temporary verification container. Review and remove retained recovery databases and old relay directories after recovery; they contain private data and consume disk. Losing the backup password makes encrypted archives unreadable.

For a lost primary, preserve the infrastructure state, backup repository, encryption password, registry artifacts, and application secrets. Recover or provision replacement compute without deleting the protected data resources. When the old primary and its deployment runners have stopped, `lk infra apply --offline --yes` can apply a saved recovery plan without contacting that host for a reservation; provider state locking and deletion checks still apply. It does not authorize deleting a protected primary. An unreachable VM that still exists needs provider-side repair or an explicitly planned replacement; this preset does not perform database failover.

On the replacement, keep the same infrastructure name, bucket, and backup repository configuration:

```sh
lk infra prepare
lk infra backups list
lk infra restore --snapshot <chosen-snapshot-id> --yes
lk infra releases
lk deploy --recover --version <compatible-recorded-tag>
lk infra backups run
lk infra status
```

`prepare` starts private services and installs the worker without starting the app or taking an initial backup. This avoids making a fresh empty database the latest backup before recovery. Choose the intended snapshot explicitly, and do not use `deploy --setup` as a replacement for this recovery sequence.

For a manual custom-format archive streamed over SSH, optionally uploaded without Restic encryption, use separate S3 credentials scoped to the backup bucket:

```sh
lk infra backup --out ../notes-backup.dump --upload
```

Existing files are never replaced; upload failure leaves the local archive available. Keep manual archives outside the project build context, and encrypt and protect them separately.

### Multiple hosts, live relay, and cache

Set `infra.instances = 2` or more and enable the existing NATS relay in the app:

```toml
[infra.services]
relay = true
cache = false

[rpc.live]
relay_url_env = "NATS_URL"
```

Build the server with the `rpc-relay` feature and configure its relay as described in [security and scale](16-security-and-scale.md#live-apps-from-one-server-to-many). Supply `NATS_TOKEN`; deployment binds the configured relay URL to the primary's private address. NATS JetStream persists relay data on that host. Cloudflare tunnel connectors run on every app host, and Kamal replaces one host at a time. New hosts need `--setup` to prepare their accessories after infrastructure apply.

`cache = true` adds a private, password-protected Valkey container and binds `VALKEY_URL`. Supply `VALKEY_PASSWORD`. It is a disposable cache bounded to 128 MiB with LRU eviction and no snapshots. App code chooses a Redis-compatible client and its own cache policy; deploying a cache does not automatically cache database queries or replace LeptosKit's live relay. Keep security-sensitive authorization fresh.

Postgres, NATS, and Valkey remain on one primary. Replicating app hosts does not provide database or broker failover. Origin HTTP ports are closed publicly; traffic arrives through the tunnel. Postgres/cache/relay bind the private interface. Ubuntu unattended security updates and bounded Docker logs are bootstrapped. Reboots, accessory image upgrades, capacity planning, and external alert delivery remain operational responsibilities. Cloudflare is an additional edge layer; it does not patch the app or host.

### Deploy on a code push

After provisioning with shared state and generating the Dockerfile, run `lk infra ci`. It writes `.github/workflows/leptoskit-deploy.yml` for pushes to `main` and manual runs. The workflow builds and pushes an amd64 image tagged with the Git commit, then runs deployment and status checks. Schema-changing releases require the manual maintenance option; initial setup has a separate manual option. Infrastructure apply remains an explicit saved-plan operation.

Use a prepared Linux runner labelled `leptoskit`, with Docker, Rust, the matching `lk` version, OpenTofu, Kamal, and an SSH agent with verified host keys and access from an allowed CIDR. Keep the runner isolated for trusted release code. Configure the generated GitHub environment, its approval/access policy, and the scoped secrets named in the workflow. The deployment registry credential must match `infra.registry_username` and be able to read the pushed image. The build job uses GitHub's package token; grant it permission to publish that repository's GHCR package. LeptosKit does not provision a CI runner or store plaintext credentials in the workflow.

For native updates, configure `codepush.server` as the updates hostname, publisher public keys, and signed releases as described in [code push](05-code-push.md). Supply R2 S3 credentials scoped to the updates bucket:

```sh
lk infra publish --platform ios --channel production
lk infra publish --platform android --channel production
```

Upload signed releases from their platform build jobs after deploying a compatible backend. Configure update-host cache rules to respect `Cache-Control`. The public updates bucket contains signed artifacts; backups stay private. Native code-push signatures, anti-downgrade checks, host-binary signing, developer accounts, and store review still apply. Direct `lk codepush publish` also supports B2 and other S3-compatible stores.

## Exporting deployment files

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

This page covers the generated files, health checks, and graceful shutdown. It deploys an `ssr` or `islands` server, or a `csr` backend named by `app.name` when `build.client_package` names its browser client. Web, iOS, Android, macOS, Windows, Linux, terminal, and HarmonyOS clients reach that server wherever it runs. A purely static `csr` app needs only `dist/` on a CDN or bucket (see [security and scale](16-security-and-scale.md#client-rendered-apps-the-cdn-is-the-server)).

## The targets

| Target       | What it writes                   | What you get                                                                                                                                                     |
| ------------ | -------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `docker`     | `Dockerfile`, `.dockerignore`    | An image for any container platform.                                                                                                                             |
| `compose`    | the above, and `compose.yaml`    | The app and its database on one machine with `docker compose up`: a Postgres service with its own volume, or a volume for the SQLite file.                       |
| `fly`        | the above, and `fly.toml`        | An app on Fly.io whose proxy checks `/readyz`, gives the server time to stop, and keeps machines running. SQLite gets a volume; Postgres is attached.            |
| `kubernetes` | the above, and `kubernetes.yaml` | A Deployment, Service, and `PodDisruptionBudget` for any cluster, with `/readyz` and `/healthz` probes and a preStop drain. SQLite gets a PersistentVolumeClaim. |
| `systemd`    | the above, and `<app>.service`   | A hardened systemd unit for a Linux host, with `/readyz` for your proxy, a 30-second stop, and the SQLite database in the service's state directory.             |

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

Secrets never go in the `Dockerfile`, `compose.yaml`, `fly.toml`, `kubernetes.yaml`, or the unit file. The Postgres URL, `AUTH_ENCRYPTION_KEYS`, and mail settings come from the environment when the container starts: `docker run -e`, an `.env` file beside `compose.yaml`, or `fly secrets set`. The `.dockerignore` keeps `.env` files, local databases, and build output out of the image.

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

## Kubernetes

`leptoskit generate deploy --target kubernetes` writes a Deployment, a Service on port 80, and a `PodDisruptionBudget` with `minAvailable: 1`. The container:

- answers `/readyz` for readiness and `/healthz` for liveness;
- runs as a non-root user with a read-only root filesystem;
- gets a 30-second termination grace period, and its `preStop` sleeps 5 seconds so the load balancer stops sending traffic before the server drains.

With Postgres the Deployment runs two replicas and reads its secrets from a Secret named after the image. With SQLite it runs one replica with a `Recreate` strategy and a `PersistentVolumeClaim` mounted at `/data`.

Build and push the image, set `image` in `kubernetes.yaml` to the pushed reference, create the Secret if the app has one, then `kubectl apply -f kubernetes.yaml`.

### One Deployment per role

An app with real-time screens can scale its parts separately: more socket servers at peak hours without more page renderers, and background jobs that never slow a page down. `--split-roles` (Postgres only) writes one Deployment per [role](16-security-and-scale.md#live-apps-from-one-server-to-many), all running the same image with `LEPTOSKIT_ROLE` set:

```bash
leptoskit generate deploy --target kubernetes --split-roles
```

| Deployment      | Replicas | `LEPTOSKIT_ROLE` | Service      |
| --------------- | -------- | ---------------- | ------------ |
| `<app>-web`     | 2        | `web`            | `<app>`      |
| `<app>-live`    | 2        | `live`           | `<app>-live` |
| `<app>-worker`  | 1        | `worker`         | none         |
| `<app>-capture` | 2        | `capture`        | none         |

Route `/live`, `/live/events` and `/live/frames` to the `<app>-live` Service in your ingress, and everything else to `<app>`. Each process checks only its own role at `/readyz`, so a socket server is ready once it follows the change log, and a worker once its workers have started. With more than one live replica, set `[rpc.live] relay_url_env` and a NATS server, so the socket servers share commits and channels through the relay; its URL, and any `[db] replica_url_envs`, are read from the same Secret as the database's. The server's `main` has to start each part only for its role, as the scaffold's does; one that ignores `LEPTOSKIT_ROLE` runs everything in every Deployment.

## systemd

`leptoskit generate deploy --target systemd` writes `<app>.service`. It runs the server from `/usr/local/bin/<app>` with the site at `/srv/<app>/dist`, reads the environment from `/etc/<app>/<app>.env`, and restarts on failure. The unit is sandboxed with a read-only filesystem, no home or devices, and only the service's `StateDirectory` writable; with SQLite, the database is `/var/lib/<app>/<app>.sqlite3`.

Create the app's user, install the binary and site, put the environment in `/etc/<app>/<app>.env`, then copy the unit to `/etc/systemd/system/`, `systemctl daemon-reload`, and `systemctl enable --now <app>`. Put a proxy that terminates TLS in front of it.

## Limits

- The image builds from the project's root. Crates outside it, such as a local leptoskit checkout passed with `--sdk-path`, aren't in the build context, so the build can't find them.
- The build copies the server from `target/release`. If `.cargo/config.toml` moves the target directory, change the `cp` line in the `Dockerfile` to match.
- The first build compiles the leptoskit CLI and every dependency, which takes several minutes. Later builds reuse Docker's caches.
- The Kubernetes manifest and the systemd unit assume the image, binary, and site are installed where they say; the generator doesn't push images or copy files to hosts.
