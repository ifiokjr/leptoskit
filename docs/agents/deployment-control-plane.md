# A simpler LeptosKit deployment product

Design investigation, 3 November 2026. The single-owner operational slice is now implemented by `lk console serve`: a durable authenticated API, one trusted repository worker, the LeptosKit dashboard and shared analytics storage. See [management service implementation](management-service.md) and the [app-author guide](../src/guide/28-deployment-service.md). Public multi-tenant hosting and Kubernetes execution below remain proposals. The original design prototype is still available separately.

## Recommendation

Make **LeptosKit Deploy** a small application platform with a control plane, a LeptosKit dashboard, and one API for agents and people. Connect once, link a repository/project, and deploy. Move installation of OpenTofu/Kamal, state configuration, operation coordination, and release bookkeeping into the service. Keep advanced configuration inspectable and exportable, but remove it from the normal workflow.

Keep OpenTofu as the infrastructure engine. A state backend alone is not this product: the missing pieces are durable jobs, runners, application releases, account connections, policy, health reporting, and an understandable interface. Pulumi illustrates the distinction: its cloud backend manages state/history, while its runner model supplies execution. [Pulumi backends](https://www.pulumi.com/docs/iac/concepts/state-and-backends/), [customer-managed runners](https://www.pulumi.com/docs/administration/concepts/customer-managed-runners/).

The user's mental model should be **project → environment → release**, with a chosen target and operating policy. OpenTofu resources, registry authentication, and rollout commands are implementation details unless the user opens advanced diagnostics.

## The normal experience

The implemented self-hosted flow is:

```text
lk console serve --project /srv/app --url https://deploy.example.com
# From the application's checkout, using a protected owner credential:
lk console connect --url https://deploy.example.com
lk deploy
```

The service owns the queue and analytics history. Existing OpenTofu state and release safeguards remain in the established deployment engine. Hosting this as a public service for unrelated customers requires the separate isolation work described below.

For a hosted service, connect a cloud account, choose a region, set a budget, and authorize routine deployments once. For self-hosting, install the same service, finish owner setup through a short-lived setup link, and connect the CLI to its URL. A scoped API token or device login links agents; cloud credentials do not belong in the repository or agent prompts. Also support existing infrastructure: attach a server or a Kubernetes cluster instead of provisioning another one.

Normal deployments that satisfy the authorized policy should proceed automatically. Ask for an exception only for a concrete boundary such as exceeding the cost policy, deleting persistent infrastructure, or a reviewed data reversal. Do not require a human click on every ordinary run.

One request starts a durable operation and returns an operation ID and dashboard URL. The CLI can follow it, disconnect, and reconnect. Losing the terminal or closing the dashboard must not cancel the deployment. The interface shows what is serving, what is changing, why a run is blocked, and the next available action.

## Architecture and ownership

```text
LeptosKit dashboard ─┐
CLI / agents / MCP ─┼─> Authenticated control API
Git events ─────────┘        |
                     Policy + durable operation journal
                         /                 \
                  Build workers        Execution workers
                       |                     |
                 OCI registry         OpenTofu provisioning
                                             |
                                     VM or Kubernetes adapter
                                             |
                                  Customer application services
```

**Control service.** A Rust service serves the LeptosKit UI and versioned API, owns projects/environments/releases, records structured events, and schedules operations. A database-backed queue is enough initially; do not require Redis or Kubernetes to run the small self-hosted control service. SQLite can support a single-process installation, with tested off-site backups; use Postgres for several controller instances and the hosted service. Keep its database separate from customer application databases.

**Execution workers.** Workers run pinned tool versions in isolated job directories and maintain durable operation ownership. Customer-managed workers can connect outbound to a hosted control plane, keeping infrastructure credentials on customer infrastructure. A hosted worker pool is an additional trust and operating model, not an automatic consequence of hosting the dashboard. Supply short-lived, scoped cloud credentials where supported; encrypt stored credentials with a key held separately from database backups. Provider plugins and build scripts can execute code. Build untrusted application code separately from privileged deployment jobs; a shared Docker socket is not tenant isolation.

**State and artifacts.** Start by reusing the existing locking S3 state backend and private object storage for plans, events, and recovery artifacts. The server configures this on behalf of each environment. Treat plans/state as sensitive. A custom OpenTofu-compatible HTTP state service could come later; implementing it is not necessary to ship a useful console. OpenTofu supports remote state, and its `remote` backend can integrate with automation services that also execute runs. [OpenTofu backends](https://opentofu.org/docs/language/settings/backends/configuration/), [remote execution integration](https://opentofu.org/docs/language/settings/backends/remote/).

**Runtime adapters.** OpenTofu owns resource lifecycles: servers, networks, buckets, DNS, and managed services. The VM adapter uses the existing Kamal runner for application replacement. A Kubernetes adapter applies application workloads and observes their health. Autoscaling belongs to the runtime controller; it should not repeatedly invoke OpenTofu. Existing exporters are a starting point, not an implemented cluster operator.

**Durability.** The server journal becomes authoritative for operations and release history, with an explicit import/reconciliation path from the current host ledger. Use idempotency keys, environment revisions, immutable image digests, and versioned plans. Persist the exact approved plan and policy decision. Recover an interrupted operation by reconciling the target before retrying a side effect. A job lease expiring is not proof its old subprocess stopped: use target-side fencing/ownership and an `unknown/reconciling` phase. Do not replace the current fail-closed lock with a timeout that can let two applies run concurrently.

Applications should continue serving when the control plane is offline. Provide an emergency CLI/recovery bundle independent of the dashboard. A co-located console cannot be its own only recovery mechanism, and updating it needs its own protected release path.

## Hosting and cost

Offer three installation patterns behind the same API:

| Pattern                                | Who runs the console                                                    | Cost consequence                                                                                                               |
| -------------------------------------- | ----------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------ |
| Hosted control plane, customer compute | LeptosKit operates the service; customers connect their targets/runners | Shares control-plane overhead across projects; a service fee and build costs still need an actual business model.              |
| One-box self-hosting                   | Console and a small app share one VM                                    | Avoids a mandatory second VM, but competes for memory and shares a failure domain. Build elsewhere and keep off-site recovery. |
| Separate control plane                 | One service manages several servers or clusters                         | Adds a base cost that can be shared across apps; isolates application hosts from console maintenance.                          |

The monthly total is control service + application compute + builds + storage/backup + traffic + domains + tax. Do not market a $5–$10 **total** for a dedicated console, redundant application hosts, and highly available Postgres. The original budget is a prototype target, not a measured bill. Avoid Kubernetes as an installation prerequisite for that target. Coolify's documented control-server minimum is already 2 CPU cores, 2 GB RAM, and 10 GB disk; this is a useful reminder to measure management overhead rather than assume it is free. [Coolify requirements](https://coolify.io/docs/start-with-self-hosted).

Show the user's budget, measured/quoted fixed charges, usage estimates, and unknown costs separately. Enforce maximum replicas/nodes and provisioning policy, but explain that limits on replica count or quoted compute do not bound all storage, traffic, or provider charges. Report drift and cost changes; never silently select a more expensive provider when capacity is unavailable.

## Deployment continuity and rollback

Aim to avoid interruptions caused by an ordinary compatible app release. Verify the immutable artifact and migrations, start a replacement with enough overlap capacity, wait for readiness and a useful smoke test, move traffic, and drain old processes. Retain assets needed by old browser clients. Long-lived connections reconnect/resume; they cannot be transferred to another process. One VM can overlap two app processes but does not provide host or database availability. The UI must distinguish deployment continuity, host redundancy, and data-service availability.

There are three different recovery operations:

- **Application rollback/roll forward:** promote a recorded compatible image digest through the same health gates. A failed new release can return traffic to a known compatible image.
- **Infrastructure change:** generate a fresh plan from the desired configuration against current state. Never restore an old state file as a universal undo button. Resizing disks, replacing hosts, and deleting data are not generally reversible.
- **Database recovery:** explicit down migrations or backup restore, with writer quiescence and reviewed consequences. A successful infrastructure or image rollback does not reverse database contents.

For online schema changes, add a real compatibility contract: separate migrations from app startup, declare the schema range each release supports, preserve checksums, and test old/new app and client versions against the expanded schema before rollout. Expand first; contract only after the rollback/client-support window. Existing migration startup rejects unknown migrations, so merely calling SQL additive does not enable safe online rollback. Keep the current maintenance path until this is implemented and proven. Down migrations are recovery tools, not the default action of the dashboard's Rollback button.

## Kubernetes and scale

Start by attaching an existing cluster through a scoped runner. Model namespaces, application Deployments/Services, readiness, resource requests/limits, and rollout status. For eligible stateless workloads, use `maxUnavailable: 0` and a bounded surge, with enough schedulable capacity. These settings alone do not prove uninterrupted traffic. Add Argo Rollouts only when canary analysis or controlled traffic splitting is needed. [Kubernetes Deployments](https://kubernetes.io/docs/concepts/workloads/controllers/deployment/), [Argo Rollouts](https://argo-rollouts.readthedocs.io/en/stable/).

Use HPA for app replica count, metrics adapters for the chosen signal, and a node autoscaler for underlying capacity. HPA needs a functioning metrics source. Avoid fighting HPA by continually reapplying a fixed replica count. Live sockets need signals such as active connections, memory, lag, and reconnect load; CPU alone may be a poor policy. Keep a nonzero live-service baseline. Database/broker scaling and high availability are separate capabilities and costs. [Kubernetes HPA](https://kubernetes.io/docs/concepts/workloads/autoscaling/horizontal-pod-autoscale/), [node autoscaling](https://kubernetes.io/docs/concepts/cluster-administration/node-autoscaling/).

The product should negotiate target capabilities. A small VM should not display a working autoscaling switch, and an attached cluster without metrics should explain what is missing. Do not promise arbitrary scale through a UI abstraction; expose tested capacity and adapter limits.

## API first, dashboard as a client

Proposed endpoints include project/environment discovery, deployment intents, plan inspection, policy approval, operation events, release promotion, target health, and backup recovery. Return typed blockers such as `budget_exceeded`, `schema_incompatible`, `runner_unavailable`, and `persistent_resource_deletion_blocked`, with an actionable next step. Events need resumable cursors, bounded logs, and redaction. The prototype's `Request` is a draft envelope with environment revision and idempotency fields; it is not an authorization implementation or a stable public API.

Agents use project/environment-scoped, revocable tokens. Browser users should authenticate with sessions; do not put infrastructure keys in browser storage. CLI credentials live in an OS credential store or protected credential file, outside project TOML. The server must independently enforce authorization and policy on every operation. A hidden or disabled button is not security. MCP can wrap this same API; it must not introduce a second approval or execution authority.

Prioritize a Vercel-like project page, current release, run timeline, readable blockers, logs, recorded image/schema compatibility, and a clear recovery action. Add providers, targets, and optional services in onboarding and advanced settings rather than filling the primary screen with raw resource configuration.

## Reuse and alternatives

Coolify and Dokploy demonstrate app-oriented self-hosted management. Dokploy documents an application, Postgres, and Traefik as core components. Terrakube provides remote OpenTofu/Terraform collaboration; Atlantis automates plans/applies from pull requests. Evaluate these as integrations or references, rather than reproduce every feature. None of these source descriptions establishes that LeptosKit's schema, native signing, or live protocols would be integrated automatically. [Dokploy architecture](https://docs.dokploy.com/docs/core/architecture), [Terrakube](https://docs.terrakube.io/), [Atlantis](https://www.runatlantis.io/guide).

The distinctive reason to build a LeptosKit control plane is its app model: Rust schema/migrations, live services, signed native updates, and compatible releases under one agent API. A generic Terraform dashboard would not deliver that integration. Keep adapters replaceable so an existing service can perform execution when that is cheaper than maintaining another subsystem.

## Delivery order and proof

1. **This iteration:** architecture and a LeptosKit console prototype. Demonstrate the workflow and honest capability/maintenance states. No cloud service is claimed.
2. **First operational slice:** a self-hosted, single-owner API with durable storage, authenticated project-scoped access, one isolated VM runner, and a complete deployment timeline. Wrap/refactor existing runner code behind a versioned operation contract. Prove lost-client and interrupted-run recovery.
3. **Plug-and-play installation:** signed/pinned installer, credential onboarding, state/bootstrap migration, repository linking, secret management, controller backups, upgrade/recovery, and measured resource overhead. A second fresh install must be repeatable without manually editing engine files.
4. **Hosted operation:** tenant isolation, quotas, accounting, audit/retention, scoped remote runners, abuse controls, incident handling, and restore drills. Do not expose a single-owner executor to multiple tenants by adding an organization ID.
5. **Online compatibility and cluster adapter:** demonstrate old/new app, schema, assets, live reconnect, and rollback under load. Then add existing-cluster onboarding, HPA and node limits, and optional canary analysis. Managed database HA and cluster provisioning are separate higher-cost products.

Before claiming deploy-without-downtime: measure failures and reconnects under traffic, failed readiness, migration incompatibility, control-plane loss, runner loss, exhausted overlap capacity, and restoration onto a replacement host. Before publishing a price: measure idle control-plane memory/CPU, build usage, object operations, transfer, and actual provider invoices. This work changes the product interface; it does not invalidate or replace the existing low-level deployment path.
