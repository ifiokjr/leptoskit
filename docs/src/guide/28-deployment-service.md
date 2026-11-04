# Your deployment service

Use one dashboard to deploy your app and see measurements from its instances. The same service accepts requests from agents and the CLI, so a deployment keeps running when you close your terminal or browser.

The service runs on a trusted Linux or macOS worker. It manages Linux application hosts through the existing deployment engine. The dashboard runs in a web browser. Native clients continue to use your deployed API and signed code-push releases.

## Start the service once

Prepare the worker with Docker, OpenTofu, Kamal 2, Git, SSH access and the provider/registry credentials required by your [managed deployment](24-deploying.md). Keep a trusted checkout of each application repository on that worker. The service is for one owner; repositories registered with it have the owner's trust.

Set `LEPTOSKIT_CONTROL_KEY` through your worker's secret store, using a random value of at least 32 printable non-space characters. Start the service:

```sh
lk console serve --project /srv/notes --url https://deploy.example.com
```

Repeat `--project` for another repository. Each repository's `infra.name` is its project ID. Each environment needs a distinct ID. The service binds to `127.0.0.1:4700` by default; put your TLS proxy or Cloudflare Tunnel in front of that address. The public origin must match `--url`. For local development, the default public URL is `http://localhost:4700`.

The dashboard, job queue and analytics history share one SQLite database. No additional database server is needed. The default private data directory is `~/.local/share/leptoskit/console`; use `--data` to select a dedicated directory. Run the command under your process supervisor so it starts after a reboot. The same host can also run the application when it has sufficient capacity.

## Connect your app

From your application's checkout, make the owner credential available through `LEPTOSKIT_CONTROL_KEY`, then run:

```sh
lk console connect --url https://deploy.example.com
```

The command checks that the project is registered, writes `infra.control_url` to `leptoskit.toml`, and saves the credential in a private file outside the repository. Commit the URL configuration; keep credentials out of source control.

Now use the normal commands:

```sh
lk infra plan
lk infra apply
lk deploy
```

The first two commands run through the service. `apply` refers to the exact plan returned to this checkout; a newer plan makes the old request invalid. Review the resource actions, your infrastructure configuration and any advanced worker plan diagnostics before applying.

For an app in Git, `lk deploy` selects the current commit. Push that commit to the registered repository's `origin` so the worker can fetch it. The worker uses a temporary checkout, builds and pushes an immutable image or verifies an existing image, then runs the normal release checks. Its infrastructure intent must match the trusted registered configuration; update that configuration and re-plan when you intentionally change infrastructure.

You can also select a prebuilt image tag explicitly:

```sh
lk deploy --version release-17
lk deploy --rollback release-16
```

Use `--setup` for the first release and `--maintenance` for a reviewed schema change, as described in [Deploying a server](24-deploying.md). Image rollback checks the recorded schema; it does not silently reverse data. Explicit database reversal and disaster recovery remain local worker operations with their existing confirmation and backup requirements.

## Use the dashboard or an agent

Open your service URL and sign in with the owner credential. The browser receives a temporary HttpOnly session cookie; the credential is not saved in JavaScript storage. The dashboard shows registered applications, infrastructure plans, deployment outcomes and application measurements.

For an agent, issue a project-scoped credential:

```sh
lk console token --project notes-production --scope deploy
```

The command prints the credential once. Store it in the agent's secret store. The available scopes are `read`, `deploy` and `ingest`. A deploy credential can submit application releases for its project. Infrastructure planning and apply require the owner credential. Ingestion credentials cannot deploy an app.

The API is under `/api/v1`. Read `overview`, submit an intent to `projects/PROJECT/operations`, then follow `operations/ID`. Each intent includes an expected project revision and an idempotency key. Repeating the same request returns the same operation. The CLI saves a pending request locally so repeating the command reconnects after a lost response.

If the service restarts during an operation, the journal marks it interrupted. Check the application's shared release/operation status and reconcile it on the trusted worker before retrying. A lost client connection does not interrupt the worker. These controls preserve uncertainty instead of automatically replaying a possibly completed operation.

## Keep analytics together

Enable the app's [analytics package](27-analytics.md) and its infrastructure plugin. Service-managed releases automatically receive a project-specific ingestion credential and exporter endpoint. These credentials grant no deployment access. Query the fleet with:

```sh
lk infra analytics
```

The service merges reporting instances without double-counting repeated batches. It retains up to seven days and 100,000 contribution windows per project, removing the oldest windows first. Queries cover at most 24 hours. The response reports the earliest retained data and stale reporting instances. Missing instances are not automatically inferred from telemetry; compare with deployment inventory.

Platform-dependent process, cgroup and filesystem samples accompany application measurements. They help investigate capacity, but do not prove database or relay saturation. Browser observations remain separate from backend observations.

## Back up and recover

Make a consistent backup while the service runs:

```sh
lk console backup --out /secure/backups/console.sqlite3
```

Store it off-site using your normal encrypted backup policy. This backs up the management journal, credentials' hashes and analytics history. Application data, provider state and signing keys have their own backup requirements.

Stop the service before restoring:

```sh
lk console restore --from /secure/backups/console.sqlite3 --previous /secure/backups/before-restore.sqlite3 --yes
```

Restore verifies a candidate database before replacing the journal, preserves the previous database, and revokes restored scoped credentials and sessions. Rotate the owner credential, reconnect agents, and redeploy applications to refresh ingestion credentials. The app can keep serving traffic while its analytics connection is being refreshed.

Management schema changes use paired up/down migrations. The offline `lk console rollback-database` command requires a new backup path and `--yes`; the first down migration deletes management history. Application migrations continue to use the separate [database workflow](22-database.md).

This service supports one trusted owner and one worker. Public multi-tenant hosting, Kubernetes execution and automatic capacity changes are separate adapters; enabling this service does not create them or guarantee availability during a host failure.

For a hosted account with a dedicated outbound worker, use [Hosted deployment accounts](29-hosted-deployments.md).
