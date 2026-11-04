# Management service implementation

The simple deployment product now has one self-hosted service, implemented in `leptoskit_deploy_console::control` and exposed by `lk console serve`. Its LeptosKit dashboard, durable operation journal, agent API and analytics collector share one SQLite database. The existing managed infrastructure engine remains responsible for OpenTofu state, provider changes, rolling application releases, database compatibility checks, backups and image rollback.

The app-author instructions are in [Your deployment service](../src/guide/28-deployment-service.md). This note records implementation choices and acceptance boundaries.

## Keep the normal path small

Prepare one trusted worker, register repositories explicitly, connect an app, then use normal `lk deploy`/`lk infra` commands. Plugin capabilities remain typed configuration blocks plus ordinary OpenTofu files. No executable plugin loader, plugin marketplace, monitoring database cluster or extra cache is needed to run this service.

Git commit releases fetch a missing commit from the registered origin, create a temporary worktree, reuse or build/push the immutable image, and release it with the existing engine. Infrastructure settings must match the registered policy; the connection URL is excluded from this comparison. The worktree shares the registered infrastructure directory, including local state and saved plans. Successful and failed attempts clean up their worktree. An abrupt worker loss can leave it behind; reconcile the run before removing that workspace.

Semantic image tags use the prebuilt-image path. This permits an external CI system to build images without giving it provider credentials. A connected generated deployment workflow needs the service credential for its deploy step; the service worker keeps provider and SSH access. For changes to infrastructure policy, update the trusted registered checkout and create a new saved plan.

## Operations and failure handling

The queue commits an intent before returning its operation ID. A unique project/idempotency constraint returns the existing operation for an identical request and rejects reuse with a different payload. Expected project revisions and a partial unique index prevent overlapping operations for the same project. One worker executes the queue serially. Clients can disconnect and later poll the same operation; they do not own its lifetime.

Apply refers to the latest successfully planned operation for that project. An older plan ID, a plan replaced by a newer attempt, or a consumed plan is rejected. The underlying engine additionally checks the saved plan fingerprint, configuration and provider lockfile and consumes the plan before apply. The UI and API show resource actions; full provider diagnostics remain in private worker logs and the inspectable saved plan.

SQLite triggers journal queued/running/final transitions and bounded progress messages atomically with operation updates. The API returns recent events with sequence numbers. At service restart, running operations become interrupted; they are never automatically replayed. The operator must inspect shared remote release history, operation ownership, schema and actual infrastructure before retrying. Existing primary-host ownership and OpenTofu locking still apply. An unexpected worker exit stops the CLI service rather than silently leaving an apparently functioning API with an abandoned queue.

## Authorization and trust

The owner credential is supplied by the process environment and compared using a digest. Scoped credentials are random, hashed, expiring, project-bound and revocable. Revocation remains a tombstone; generating a deterministic managed ingestion credential does not silently reactivate a revoked value. Rotate the owner credential to derive a new managed ingestion value after revocation.

The browser exchanges an owner credential for an eight-hour HttpOnly, SameSite=Strict session cookie. HTTPS origins add Secure. Browser mutations require the configured Origin. Bearer clients do not use browser sessions. Read and deploy keys are separate from ingestion keys; provider planning/apply require owner authority. API callers cannot submit paths, environment variables, arbitrary executables or shell commands. Restore revokes all restored tokens and sessions; rotate the owner key and redeploy apps to refresh derived app keys.

This is a single-owner trusted-repository service. The worker runs under that owner's account with provider access and a Docker daemon. Separate Git worktrees provide source separation, not a sandbox for hostile code. This is not a multi-tenant executor or an isolation boundary between unrelated organizations. Do not turn it into one by exposing the API and adding organization IDs.

## Storage and analytics

The private database uses WAL, full synchronous writes, transactional paired migrations and a migration checksum. Unknown schema versions and modified applied SQL fail startup. Consistent backups are written to a temporary file then published without overwriting an existing backup. Offline restore checks a candidate's integrity and foreign keys, retains the current database, and revokes restored credentials before publishing the replacement. Offline down migration requires a fresh backup; its destructive effect is explicit.

SDK exports have one in-flight request every 30 seconds and no unbounded retry queue. The service validates identities, clocks, routes, window counts, numeric bounds and histogram consistency. Contributions are keyed by project, producer, series and window. Higher-count updates replace the same contribution, while late lower-count snapshots cannot regress it. A producer's release cannot change without a new boot identity. Fleet queries sum distinct contributions, preserving source and release dimensions.

History retains at most seven days and 100,000 contribution windows per project; oldest windows expire first. Queries are bounded to 24 hours and expose retained coverage and stale reporting instances. The journal keeps the latest 10,000 operations. The SQLite file has an approximately 1 GiB page limit. Local debug logs are private, capped at 4 MiB per stream and periodically pruned to roughly 64 retained streams. These are storage bounds, not a resource-use benchmark or a guarantee of a particular monthly bill.

Host samples report their process/cgroup/filesystem scope and omit unavailable values. They are not relabeled as complete VM, database-pool or NATS telemetry. Browser signals remain untrusted and separate from backend observations. Snapshots arriving after an outage only cover the SDK's retained memory window; unexported observations can be lost on an app crash. Fleet counts cover reporting instances, not the expected deployment inventory.

The optional OTLP adapter emits completed delta histograms through OTLP/HTTP JSON, with resource identity, explicit bucket bounds and string-encoded uint64 fields. Partial rejects are export failures. Ambiguous network retries may repeat a delta; the receiving backend needs timestamp deduplication. This adapter does not claim a full span graph, automatic database/RPC hooks or session replay.

## Validation and remaining boundaries

Tests cover durable queue replay/idempotency, revision conflicts, exact-plan ownership, schema up/down, consistent backup/restore, token revocation, browser session origin enforcement, project scopes, deduplicated fleet history and a real HTTP path from an application exporter to the durable collector. The CLI service test runs a real daemon/client exchange and the existing release engine with mocked cloud/SSH tools after the original request disconnects.

No real cloud provider acceptance deployment, host-failure availability test, public hosted multi-tenant service or Kubernetes executor is claimed. Those are separate targets from completing this small self-hosted workflow. The original console design prototype remains a separate example and does not represent capabilities that the live worker lacks.
