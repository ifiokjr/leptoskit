# Infrastructure and deployment exploration

Research date: 2 November 2026. This records the original proposal. A managed deployment implementation is now available; its supported commands and limits are documented in [managed deployment](../src/guide/24-deploying.md#managed-deployment). The wider architecture below remains research. No cloud resources were created. Prices are published list prices checked during this investigation, not account-specific quotes or capacity guarantees.

## Implementation status

The managed implementation now covers Hetzner and DigitalOcean compute, private networking, Cloudflare tunnels and signed-update delivery, optional Valkey/NATS, multi-host application replacement, shared release history and reservations, schema-aware builds and rollback, Restic scheduling/retention/restore verification, R2 or B2 backups, provider-price discovery, an optional compute-price guard, and an app-owned GitHub workflow. See `crates/leptoskit_cli/src/infra/` and `crates/leptoskit_config/src/infra.rs`. The proposal below describes the broader design and the repository baseline at investigation time, not a second configuration API.

Generated resources have been validated locally with OpenTofu 1.13.1, hcloud 1.70.2, Cloudflare 5.28.1, DigitalOcean 2.106.0, and B2 0.14.1. Runtime configuration has been checked with Kamal 2.10.1, NATS configuration with nats-server, and the generated workflow with actionlint. Tests exercise real PostgreSQL/SQLite migration reversal, real PostgreSQL + encrypted Restic backup/restore using a local Docker transport adapter, authenticated NATS connections, and CLI failure/recovery boundaries. These checks do not provision a VM, run real Docker containers through Cloudflare, measure live reconnects during cloud deployment, verify account capacity/billing, or prove restore onto a replacement cloud host. Those remain credentialed acceptance checks. Database/broker failover, managed hosting on arbitrary providers, native store automation, state-backend bootstrap, and schema-compatible online migrations remain outside this preset.

## Recommendation

Make LeptosKit the interface for deploying a complete application: Rust application and database schema, TOML infrastructure intent, a Rust deployment CLI, and established engines underneath. Start with one Linux VM, Cloudflare in front, an always-running application, a local database, and off-site backups. Support native release distribution through the same release manifest, while retaining each store's signing and review requirements.

Use **OpenTofu for provisioning** and evaluate **Kamal for application releases**. Generate OpenTofu JSON from typed Rust structures, and generate Kamal YAML from the same resolved deployment specification. Users should not need to write either language for the supported presets. Keep the generated files inspectable and exportable. Do not build a new general-purpose infrastructure reconciliation engine.

A $5–$10 monthly infrastructure starting point is plausible for a small application. It buys one failure domain and limited resources. It does not buy database failover, uninterrupted host maintenance, unlimited traffic, or an operations team. Resource overlap during deployment needs memory headroom. Build Rust/Wasm images in CI or on a development machine rather than on the small production VM.

## What the repository already supplies

These are observations from this checkout, not proposed APIs:

| Foundation            | Evidence                                                                                                                        | Implication                                                                                                                                                                           |
| --------------------- | ------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Deployment generation | [`crates/leptoskit_cli/src/deploy.rs`](../../crates/leptoskit_cli/src/deploy.rs)                                                | Generates Docker, Compose, Fly, Kubernetes, and systemd artifacts; does not yet manage cloud resource lifecycles.                                                                     |
| Health and shutdown   | [`docs/src/guide/24-deploying.md`](../src/guide/24-deploying.md), [`leptoskit_axum`](../../crates/leptoskit_axum/src/health.rs) | `/readyz`, `/healthz`, and graceful draining can support a release runner. Generated platform stop timeout is 30 seconds against a documented 25-second application shutdown default. |
| Portable schema       | [`leptoskit_db`](../../crates/leptoskit_db/readme.md)                                                                           | Rust table definitions and migrations already target SQLite and Postgres. Database provisioning should reuse these rather than create another schema system.                          |
| Live hub              | [`hub.rs`](../../crates/leptoskit_rpc/src/server/live/hub.rs)                                                                   | One change-log reader per server; identical subscriptions share query work within the same caller identity. Redis is not needed to obtain this benefit.                               |
| Relay                 | [`relay/mod.rs`](../../crates/leptoskit_rpc/src/server/relay/mod.rs)                                                            | NATS JetStream already carries commits between servers; leased publishing, deduplication, gap recovery, and database fallback exist.                                                  |
| Role separation       | [`security and scale`](../src/guide/16-security-and-scale.md)                                                                   | The same image can run web, live, jobs, and capture roles; Kubernetes generation already supports split roles on Postgres.                                                            |
| Real example          | [`Huddle`](../../examples/huddle/readme.md)                                                                                     | Exercises SQLite, Postgres, and two servers through NATS. Local benchmark results establish useful evidence, but do not establish throughput on a cheap VM.                           |

Two concrete gaps matter before promising fully managed deployment:

1. Deployment generation currently refuses client-rendered apps with a separate backend; Huddle documents a workaround. Model the presence of a backend independently from the web render mode.
2. [`migrate::pending`](../../crates/leptoskit_db/src/migrate.rs) rejects applied migrations unknown to the binary. After a schema-changing release, the old image can fail at startup even if the change was additive. Image rollback alone is currently insufficient.

## Infrastructure engine choice

| Option                           | Assessment for LeptosKit                                                                                                                                                                                                                                 |
| -------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| OpenTofu                         | Preferred default. Declarative resources, provider ecosystem, plan/apply, state, import, drift detection, and encryption. Its JSON configuration is explicitly intended for programmatic generation. Rust can emit it with ordinary serialization.       |
| Terraform                        | Reasonable export/enterprise integration where teams already standardize on it. HCL/JSON is not a Rust obstacle. OpenTofu is the cleaner default for an open toolkit, but provider and language compatibility still needs versioned tests.               |
| Pulumi                           | Credible alternative, especially for teams wanting programmable infrastructure. Official SDKs cover several languages and YAML; Rust is not on the supported-language list. TypeScript is optional, not mandatory. A generated YAML backend is possible. |
| Rust IaC libraries               | `terrars` demonstrates generating Terraform configuration from Rust. Interesting for advanced users, but not a necessary dependency for a constrained TOML interface. Provider coverage and maintenance need evaluation before adoption.                 |
| A new Rust reconciliation engine | Avoid initially. Partial failures, imports, provider changes, deletes, locking, secret-bearing state, drift, and recovery are the expensive work. A TOML parser does not solve those problems.                                                           |
| CDK for Terraform                | Exclude from a new design: HashiCorp deprecated it on 10 December 2025.                                                                                                                                                                                  |

Sources: [OpenTofu JSON](https://opentofu.org/docs/language/syntax/json/), [state encryption](https://opentofu.org/docs/language/state/encryption/), [Pulumi languages](https://www.pulumi.com/docs/iac/languages-sdks/), [Pulumi YAML](https://www.pulumi.com/docs/iac/languages-sdks/yaml/), [terrars](https://docs.rs/terrars/latest/terrars/), [CDKTF deprecation](https://developer.hashicorp.com/terraform/cdktf/release).

TOML is a good product interface because LeptosKit can constrain valid combinations, explain costs, and validate capabilities. It is not inherently easier for an agent than TypeScript or HCL. The advantage comes from a small schema, predictable defaults, useful errors, deterministic plans, and explicit release semantics.

## Provisioning, host management, and releases are separate responsibilities

OpenTofu should own machines, firewalls, DNS, tunnels, buckets, and cloud identities. An initial cloud-init configuration can bootstrap the host, but cloud-init alone is not ongoing patch management.

A host policy must own operating-system updates, container runtime updates, disk limits, log rotation, service supervision, and controlled reboots. Ansible or a maintained image with systemd timers can cover the first version. NixOS plus deploy-rs is attractive for reproducible host generations and host rollback, particularly given the repository's existing Nix use, but adds a second ecosystem to support. Host rollback still does not reverse database contents. See [deploy-rs](https://github.com/serokell/deploy-rs).

The release runner owns image digests, preflight checks, migrations, readiness, switching traffic, draining, and release history. Keep app deployments from inadvertently replacing infrastructure. Avoid putting every release into an OpenTofu remote-exec provisioner.

Kamal is a strong first integration candidate: it deploys containerized applications to ordinary VMs and has proxy health switching and image rollback. It introduces a Ruby dependency in the operator/CI environment, not the application. Validate its private networking and WebSocket drain behavior before committing to it. Do not confuse its rollback command with an automatic health-based rollback policy; LeptosKit still needs to define the latter. Sources: [deploy](https://kamal-deploy.org/docs/commands/deploy/), [configuration/timeouts](https://kamal-deploy.org/docs/configuration/overview/), [rollback and retention](https://kamal-deploy.org/docs/commands/rollback/).

Alternatives: retain systemd for a minimal single-process preset; offer Docker/Compose export for people operating their own host; integrate existing Coolify or Dokku installations rather than requiring a dashboard control plane on every small VM. Coolify explicitly excludes Compose applications from its application-level rolling update path, and warns that overlapping containers alone does not guarantee zero downtime. Kubernetes belongs in an advanced adapter, not the $10 starting point. [Coolify rolling updates](https://coolify.io/docs/applications/deployments/rolling-updates).

## Smallest useful topology

```text
Web and native clients
        |
Cloudflare DNS / TLS / WAF
        |
Cloudflare Tunnel (outbound connection from the VM)
        |
Local release proxy
        |
LeptosKit web + API + live + jobs in one application
        |
SQLite OR private local Postgres

Private object storage: uploads, signed releases, backups
Optional later: Valkey cache; NATS when several app processes share live state
```

Cloudflare is an edge service configured through an account and zone; no Cloudflare VM needs provisioning. A named tunnel plus DNS record can be provisioned through its provider. `cloudflared` connects out, so the application does not need an internet-facing origin port. Administration must have its own defined access path; close inbound traffic only after that path works. IPv6-only VMs require verified IPv6 connectivity for management, image pulls, and every external dependency; NAT/IPv4 fallback can add cost.

Use private networks and no published database, Valkey, or NATS ports. The release proxy must also stay private. A direct-origin alternative needs firewall restrictions and authenticated TLS to prevent bypassing Cloudflare. Do not rely only on hiding the IP address. Sources: [Tunnel](https://developers.cloudflare.com/cloudflare-one/networks/connectors/cloudflare-tunnel/), [provisioning tunnels](https://developers.cloudflare.com/tunnel/guides/terraform/).

Cloudflare Free supplies a subset of managed WAF rules. WebSockets are supported, but WAF inspection applies to the opening HTTP request, not every subsequent message. Application authorization, message validation, connection quotas, and rate limits remain necessary. Cloudflare may terminate connections during edge updates, so reconnect/resume must work independently of deployment. It cannot protect a compromised dependency after execution or replace patching. Sources: [WAF availability](https://developers.cloudflare.com/waf/get-started/), [WebSockets](https://developers.cloudflare.com/network/websockets/).

Cache fingerprinted assets and immutable release blobs. Never cache authenticated HTML, session endpoints, private API responses, or live streams as public content. Avoid browser challenges on native API traffic. Use Cloudflare Access for operator interfaces where appropriate, not indiscriminately in front of the public application API.

## Cost evidence and provider selection

Published prices, excluding taxes and unrelated services:

| Candidate                       | Published starting point relevant here                                                | Assessment                                                                                                                      |
| ------------------------------- | ------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------- |
| Hetzner CX23 in Germany/Finland | $6.49 / €5.49 per month excluding IPv4; 4 GB RAM                                      | Preferred value candidate if available in an acceptable region. Verify price and stock through the account API before planning. |
| DigitalOcean Basic              | $6/month for 1 GiB; $12 for 2 GiB                                                     | Straightforward alternative; much less memory at the $6 tier. A small Rust/SQLite app may fit, but overlap needs measurement.   |
| Fly Machines                    | Example table: $5.92/month for shared CPU/1 GB, or $3.32 for 512 MB; region dependent | Best first hosted adapter because generation already exists. Add volume, bandwidth, and other applicable charges.               |
| Railway Hobby                   | $5 minimum, including $5 usage                                                        | Good convenience option. Usage above the allowance is extra; the plan is not a $5 all-in app/database/cache package.            |
| Lightsail                       | Fixed bundles with IPv4/IPv6 differences                                              | Worth a supported alternative where AWS accounts/regions matter; quote the exact bundle and address type.                       |

Sources: [Hetzner prices](https://docs.hetzner.com/general/infrastructure-and-availability/price-adjustment/), [Hetzner specifications](https://www.hetzner.com/cloud/cost-optimized/), [DigitalOcean](https://www.digitalocean.com/pricing/droplets), [Fly](https://fly.io/docs/about/pricing/), [Railway](https://railway.com/pricing), [Lightsail](https://aws.amazon.com/lightsail/pricing/).

Illustrative low-traffic budget: $6.49 VM + roughly $0.14 for 20 GB of B2 storage at $6.95/TB-month, conservatively ignoring free storage allowances = **about $6.63/month**. Cloudflare Free adds no plan subscription. This is a feasibility calculation, not a tested deployment quote. It assumes IPv6-only connectivity works, no charged egress, no paid registry/build service, and no other add-ons. Domain registration, taxes, email/SMS, AI APIs, payment fees, and operator time are additional. Backup growth and uploads count toward storage. Allowing for optional address/backup charges and modest usage makes $7–$10 a sensible target rather than a guarantee.

“Find the cheapest provider” should mean **choose the cheapest eligible plan from a curated, tested catalog**. Filter by region, architecture, memory, persistent disk, traffic, credentials, stock, and provider capability. Include temporary release overlap, taxes, IPv4, backup storage, and registry costs in the estimate. Show assumptions and a timestamp; refresh quotes before apply. Stop if no eligible plan fits the cap rather than silently selecting an expensive fallback. Never automatically move a live database to another provider just because its list price falls.

A budget field can constrain planned resources. It cannot enforce a universal hard invoice cap against variable bandwidth, object storage, or third-party charges. Add provider billing alerts and explicit runtime quotas. Marketing a genuinely fixed monthly subscription would require LeptosKit to resell/host the infrastructure and absorb usage risk; bring-your-own-account IaC gives users several provider bills.

## Database and backup defaults

Offer two honest presets:

- **Smallest app:** SQLite on persistent local disk, one active application process, continuous off-site backup, and a short controlled restart during releases. Do not promise seamless overlap for arbitrary SQLite applications.
- **App with overlapping releases:** Postgres as a persistent service independent from the application containers, with old/new app containers sharing it during a release. This can still fit a small 4 GB VM at low load, subject to measured memory limits. It remains one host with no automatic database failover.

Prefer Postgres for the preset that advertises rolling/overlapping app releases. Keep SQLite attractive for the cheapest prototype. Do not imply swapping a database URL performs a SQLite-to-Postgres data migration: matching schema definitions help, but moving data, validating types/sequences, and coordinating cutover need a separate workflow.

Use Litestream as a candidate SQLite backup process; its asynchronous WAL replication is disaster recovery, not synchronous database HA. Use database-consistent Postgres backups, initially scheduled logical backups with an explicit recovery-point objective; add continuous WAL archiving/PITR for stronger recovery needs. Never copy a live SQLite file or Postgres data directory casually and call it a valid backup. Sources: [Litestream internals](https://litestream.io/how-it-works/), [restore/file replica](https://litestream.io/guides/file/).

Default to encrypted off-site backups with retention, a separate recovery key, and periodic restores into a temporary database. Record backup age, successful restore age, and measured recovery time. A VM snapshot alone is not enough. Keep backup credentials separate from app upload credentials, and prefer protection against deletion by a compromised app. Do not automatically restore a backup on an application rollback: doing so can erase legitimate new writes.

## Object storage: B2 and R2

B2 is a good default for off-site backups and bulk storage: published storage starts at $6.95/TB-month, free transactions, and free egress up to three times average stored data. Qualifying partner routes, including Cloudflare, have separate free-egress benefits. Do not assume every direct download from a native client or backup restore qualifies. Backblaze's provider manages buckets and application keys. Sources: [pricing](https://www.backblaze.com/cloud-storage/pricing), [provider guide](https://www.backblaze.com/docs/cloud-storage-use-backblaze-b2-terraform).

R2 is a strong alternative for app uploads and release delivery: Standard storage is $0.015/GB-month, with monthly free allowances and no internet egress charge; request charges remain. At prototype storage volumes, free allowances can matter more than the per-TB price. Fewer accounts also have operational value. [R2 pricing](https://developers.cloudflare.com/r2/pricing/).

Expose an S3-compatible object API in Rust with explicit capabilities: upload/download, metadata, multipart uploads, and signed URLs. Endpoint, bucket, region, credentials, and path-style behavior come from deployment bindings. Test B2 and R2 separately; “S3 compatible” does not mean every ACL, conditional request, CORS, retention, and encryption feature behaves identically. Serve private objects through signed access; never turn an entire bucket public to simplify integration. Prefer separate logical buckets/credentials for uploads, backups, and signed code-push artifacts.

## Redis, Valkey, NATS, and live performance

Do not install Redis by default. On one server, bounded in-process caches avoid network overhead and another service. The current live hub already shares query/diff work within an identity. Measure subscriber fan-out, query rerun time, database CPU, memory per connection, and reconnect storms before adding infrastructure.

Use explicit abstractions rather than a universal “memory database”:

| Need                | Initial implementation        | Scale path                                |
| ------------------- | ----------------------------- | ----------------------------------------- |
| Read cache          | Bounded local cache with TTL  | Valkey/Redis-compatible cache             |
| Durable app data    | SQLite/Postgres               | Postgres with suitable replicas/HA        |
| Live commit fan-out | Existing local hub            | Existing NATS JetStream relay             |
| Durable jobs        | Existing database-backed jobs | More job workers, with idempotent effects |
| Presence/signals    | Existing local implementation | Existing relay across app instances       |
| Global rate limits  | Explicit app policy           | Shared store with atomic operations       |

Valkey is a BSD-licensed, Linux Foundation-backed in-memory datastore and a sensible optional cache target. Set max memory, eviction policy, TTLs, ACLs, private networking, and client timeouts. A cache should tolerate loss; security-sensitive throttles need a deliberate failure policy. Auth revocations and live permissions must not become stale just because a TTL has not elapsed. [Valkey](https://valkey.io/).

Upstash can remove cache operations work. Its current free tier advertises 500K commands/month; pay-as-you-go advertises $0.20/100K commands. Chatty live workloads can consume that quickly: 100 commands/sec continuously is about 259.2 million commands per 30-day month, or $518.40 in command charges before other considerations. Use co-location, batching, quotas, and workload estimates rather than assuming serverless caching is always cheapest. [Upstash pricing](https://upstash.com/pricing/redis).

Keep NATS because it is already integrated and supplies the messaging semantics the relay expects. Redis Pub/Sub would not be a drop-in durable replay substitute; adopting another transport means implementing and testing equivalent recovery behavior. JetStream supports persisted replay and at-least-once consumers; LeptosKit's own version/deduplication logic is still part of correctness. Bound stream bytes/age and process resources; a single NATS container is not a replicated cluster. [JetStream](https://docs.nats.io/concepts/jetstream).

## Long-running services and release safety

Always-on VMs/containers are the straightforward fit for the existing native Rust server and live hub. Disable scale-to-zero in the live preset. Support idle connections with heartbeats and align edge/proxy/app timeouts. Cloudflare Durable Objects are a valid counterexample to “serverless cannot do live”: their WebSocket hibernation model can support persistent clients. However, using it here would be a different runtime/storage architecture, not a transparent deployment target for the current server. [Durable Objects pricing/model](https://developers.cloudflare.com/durable-objects/platform/pricing/).

Release procedure for the Postgres overlap preset:

1. Build/test once and publish an immutable image digest plus web assets; record source revision, schema version, protocol range, and native/code-push artifacts.
2. Acquire a deployment lock; verify disk, memory overlap, backups, secrets, and the exact infrastructure plan where needed.
3. Apply an explicitly reviewed, backward-compatible schema expansion in a dedicated migration step. Keep the old app serving.
4. Start the new app on a private endpoint with the expected configuration; check readiness and a representative authenticated HTTP/live smoke journey.
5. Switch new requests/connections through the local release proxy. Existing WebSockets remain on the old process until drained; they cannot migrate between processes.
6. Drain for a bounded period and close remaining sockets with retry semantics; clients reconnect with jitter and resume versions, or request a fresh snapshot when retention has expired.
7. Observe errors/reconnect failures/latency. Revert traffic to the prior compatible image when policy permits. Retain image digests/assets long enough for rollback and old browser clients.
8. Remove old schema only in a later release after the rollback window and supported client window close.

On one host this is overlapping application replacement, often called blue/green; rolling updates across hosts arrive with a multi-host preset. Host failure, reboots, database restarts, and tunnel/proxy updates can still interrupt service.

Fix migration compatibility before advertising rollback: separate applying migrations from starting the app; define which newer schema versions an older release can serve; retain checksum verification for migrations it knows; fail closed outside the declared range. Simply ignoring all unknown migrations would weaken an existing integrity safeguard. Test old/new binaries against expanded schemas before switching. A rollback that cannot safely run must report the reason and retain a forward-recovery path.

Also test old/new live roles simultaneously: relay event envelopes, query protocol versions, subscription IDs, auth revocation, reconnects, and background jobs. An old mobile app can remain installed for months, so compatibility needs a supported client window beyond the immediately previous server release.

## Proposed TOML and CLI

The entire example below is illustrative; these sections and commands do not currently exist. Preserve existing `[db]`/`[rpc]` definitions and add deployment bindings instead of declaring the same database twice.

```toml
[deploy]
environment = "production"
preset = "small-postgres"
domain = "app.example.com"

[deploy.compute]
provider = "hetzner"
region = "hel1"
size = "cx23"
instances = 1

[deploy.budget]
currency = "USD"
monthly_target = 10
reject_plan_above_target = true

[deploy.edge]
provider = "cloudflare"
connection = "tunnel"
plan = "free"

[deploy.database]
placement = "local-service" # provisions the existing [db] database
backups = "continuous-or-scheduled" # resolved by database capability

[deploy.storage.backups]
provider = "b2"
private = true
retention_days = 30

[deploy.release]
strategy = "overlap"
retain = 5
drain_seconds = 30
rollback = "compatible-only"

[deploy.cache]
provider = "none"

[deploy.live]
relay = "none" # multi-instance presence requires NATS in the first version
```

Resolve the loose backup policy to an explicit schedule/RPO in the plan; it must never remain ambiguous at apply time. Real credentials belong in an operator/CI secret store, not TOML. Account/zone identifiers can be config values. Provider-specific settings remain optional escape hatches and must be validated against the selected adapter.

Proposed workflow:

```text
lk infra plan --environment production --json
lk infra apply --plan <saved-plan>
lk deploy --environment production --image <immutable-digest>
lk releases list --environment production
lk rollback --environment production --release <release-id>
lk backup verify --environment production
lk infra drift --environment production
```

The agent should get machine-readable plans, cost assumptions, health results, and actionable error codes. Authorization can permit routine deployments in advance. Destructive data operations and spending beyond the authorized policy must remain distinct actions; an interactive confirmation on every ordinary release would defeat agent automation.

## State, credentials, and interrupted runs

Use a protected remote state backend with locking, versioning, and encryption. Bootstrap it separately from the main stack so losing the main VM does not lose the infrastructure map. Keep the recovery key outside the machine and outside encrypted state itself. Pin engine/provider versions and generated-module versions. Do not commit state or secret-bearing plans.

OpenTofu's S3 backend supports native lock files through conditional writes. Validate the exact selected object store's behavior before claiming compatibility; S3 API compatibility and immutable Object Lock are not proofs of concurrent state locking. Choose a tested backend rather than automatically reusing the application's B2 bucket. [S3 backend locking](https://opentofu.org/docs/language/settings/backends/s3/).

Record resource ownership and release phases. A retry after partial provisioning or a lost connection must converge, not create another server or lose track of a bucket. Support importing/adopting existing resources. Keep logs redacted, distinguish operator credentials from app credentials, and scope tokens to the relevant project/zone/bucket. Protect data resources from routine teardown. The release record should include configuration and secret-version references, never plaintext secrets.

## One codebase across platforms

Use one project-level release manifest with separate deployable artifacts: server image, web assets, signed native Wasm/code-push bundles, and host-app binaries. Stable API/live endpoints are runtime bindings shared by the clients. Upload signed release files before updating the release index, and keep earlier blobs available during the rollback window.

Cloud provisioning can be automated with provider credentials, but native distribution still needs platform build tools, signing credentials, developer accounts, entitlement configuration, store policies, and review. Code push does not replace a host binary update when native functionality changes. Existing [`publishing-plan.md`](publishing-plan.md) is the place to align that workflow. The product promise should be one project and one coordinated release workflow, with truthful per-platform status and recovery controls.

## Implementation order and proof gates

1. **Prove the existing app path.** Deploy Huddle with Postgres to one VM behind a tunnel using existing image generation and an external runner. Measure idle/load/overlap memory and real invoice components. Keep this a deliberately small integration experiment.
2. **Make rollback truthful.** Add schema compatibility and a dedicated migration phase. Exercise live reconnect and auth revocation during release and rollback. Fix backend discovery for client-rendered apps.
3. **Introduce the typed deployment specification.** Generate schema/documentation and deterministic OpenTofu JSON. One compute adapter, Cloudflare, and B2/R2 first; plan/apply/state recovery and idempotence are required together.
4. **Add operational defaults.** Host patch policy, bounded logs, disk alerts, off-site backups, restore verification, redacted logs, and a release history that survives losing the host.
5. **Add optional services and scale.** Valkey when measurements justify shared caching; NATS when multiple app processes need shared live state; multi-host rolling releases and managed Postgres as separate, higher-cost presets.
6. **Coordinate native delivery.** Reuse publishing/code-push foundations, release compatibility windows, and per-platform result reporting.

The first end-to-end acceptance test should provision an empty account/project, migrate, serve an authenticated live app through Cloudflare, deploy a compatible update under load, reconnect clients, roll back, restore a backup onto a replacement host, and clean up without deleting protected data. Include failed readiness, dropped relay, lost deployment connection, full disk, and exhausted capacity. Record observed results rather than claiming theoretical zero downtime.

The largest unresolved engineering choices are migration compatibility, release-runner networking/draining, provider/state compatibility, host lifecycle ownership, and actual VM sizing. None prevents the overall design. Together they define the work between generating deployment files and responsibly managing a deployed application.
