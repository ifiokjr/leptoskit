# LeptosKit deployment service

<!-- {=deploy_consoleSummary} -->

One optional service hosts the dashboard, durable operation queue, scoped agent API and analytics history. It uses SQLite and reuses the existing `OpenTofu`/Kamal engine through fixed CLI operations. No Redis or separate monitoring database is required.

<!-- {/deploy_consoleSummary} -->

```sh
# On a trusted worker, with LEPTOSKIT_CONTROL_KEY and provider secrets configured:
lk console serve --project /srv/app --url https://deploy.example.com
# From the app checkout, with its management credential available:
lk console connect --url https://deploy.example.com
lk infra plan
lk infra apply
lk deploy
```

The worker needs Docker, OpenTofu, Kamal 2, Git and SSH. Configure TLS or a Cloudflare Tunnel in front of the default localhost listener. Registered repositories share one owner's trust and worker credentials; this is not a public multi-tenant executor. A normal Git-commit release uses a temporary checkout and an immutable image; prebuilt image tags are supported too.

The `server` feature enables `control::{Service, Store, Executor}`. The dashboard is a real LeptosKit builder tree, rendered on the server with a small browser interaction module. API and browser requests use the same operation contract, authorization, revision and idempotency checks. Browser sessions use HttpOnly cookies. Worker output remains in bounded private files; the API exposes typed status, progress events and reviewed resource actions.

The app analytics exporter sends bounded snapshots to a project-scoped ingest endpoint. The store upserts per-producer windows and aggregates reporting instances without counting retries twice. Its paired migrations, online backup, offline restore and schema rollback are exposed by `lk console`. Restored tokens are revoked. See the public deployment-service and analytics guides for setup, retention and recovery limits.

The original design study is still available through `model`, `store`, `view` and the `preview` example. It deliberately uses fixture data and does not issue cloud operations. Its target/availability previews are separate from the live single-owner service; Kubernetes and hosted multi-tenant execution remain separate adapters.

Validation runs through the repository's devenv shell:

```sh
cargo test -p leptoskit_deploy_console --all-features
cargo test -p leptoskit_cli --test infra
cargo check -p leptoskit_deploy_console --target wasm32-unknown-unknown --features csr
```
