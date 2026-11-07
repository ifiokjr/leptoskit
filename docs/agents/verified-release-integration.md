# Verified backend image integration (A04)

This is an opt-in backend-only path. Existing image builds remain legacy. It does not add signatures, reuse deployment-policy signatures, or reuse code-push approvals. An independently authenticated descriptor digest is the operator's trust root; a publisher-supplied JSON file and its adjacent hash are not authentication. SSH image observations are not runtime attestation.

## Commands

The recipe and inventory use the existing backend verification format. Select real builder and runtime digests independently; do not substitute tags.

```sh
lk verify build --recipe recipe.json --out verified-build
lk verify reproduce --bundle verified-build/bundle --manifest verified-build/inventory.json --sha256 "$INVENTORY_PIN"
lk infra build-verified --bundle verified-build/bundle --manifest verified-build/inventory.json --sha256 "$INVENTORY_PIN" --runtime-image "$RUNTIME_IMAGE_AT_DIGEST" --runtime-config-sha256 "$RUNTIME_CONFIG_PIN" --tag registry.example/backend:unique-version --out descriptor.json
```

Packaging performs full reproduction again; it does not import a reproduction claim from JSON. Offline reproduction and supplied checkouts remain useful verification commands, but cannot authorize managed packaging. Runtime config pins hash the compact, recursively key-sorted JSON of the engine inspect object's Config member (SHA-256). Select that configuration deliberately: environment, healthcheck, shell and other inherited settings remain bound to the pin. ONBUILD and volumes are rejected.

The private packaging context contains only the executable and generated Dockerfile. Its instructions are FROM, COPY, WORKDIR, USER, ENTRYPOINT, CMD. No application Dockerfile or RUN instruction is accepted. Image inspection checks the complete inherited/modified configuration and platform. Extraction checks executable bytes and mode 0755. The image filesystem is also checked for unsupported /app/services. No descriptor is written on a failed check. The successful descriptor reports an engine image ID, not a registry digest.

The same image/configuration/extraction checks apply with `--reuse-existing`, which inspects an existing local tag instead of building it.

Publication is a separate operator action; acceptance testing must not push. After deliberately publishing the checked image, authenticate the descriptor digest independently, then deploy:

```sh
lk deploy --version unique-version --verified-descriptor descriptor.json --descriptor-sha256 "$DESCRIPTOR_PIN"
lk deploy --rollback previous-version
lk deploy --recover --version previous-version
```

Use existing setup/maintenance/rollback-schema flags as required. Those checks are unchanged. History records the descriptor together with the observed registry digest. Pulling/reusing a recorded tag must match both; new releases check the independent descriptor before traffic changes and again before history advances. Once history contains verified evidence, it cannot downgrade to a legacy record. Legacy history deserializes with absent evidence, never invented verification. Shared history retains the existing operator trust model; this is not an append-only signed transparency log.

## Scope and acceptance gaps

Only projects whose sole platform is server are accepted. Schema fingerprints come from migrations inside the frozen verified source. Source, recipe, dependencies and every bundle entry are covered by the independent inventory. Website assets and service-bearing projects are rejected. PR #553 requires a later descriptor covering every service executable and schema; app-only evidence must not authorize it.

RPC export runs the checked image executable with no network, a read-only root, no capabilities, a non-root user, bounded resources, and a 30-second deadline. The executable must export before database/auth startup; arbitrary executable behavior is not attested. The descriptor hashes the exact exported spec bytes after parsing. Compatibility is explicitly not_checked. Only an explicit `--no-rpc` operator declaration records RPC as not_applicable. Native/guest compatibility and assets are not_applicable; services are unsupported; runtime attestation is absent. No online schema compatibility claim is made.

The real-engine fixture acceptance remains pending: a tiny committed locked backend, independently pinned builder/runtime, build twice, full reproduce, package without push, extract/hash, then reject tampering. Do not mark this acceptance complete from fake-engine or unit tests. The local engine was unavailable on Mini02 and no machine was provisioned alongside compilation.

## Mini02 implementation evidence

- Base revalidated against remote main: f198f496.
- CLI library suite: initial run 547 passed, 5 ignored. After adding connected deployment and scope checks, 548 passed and one new fixture failed due to malformed test TOML. The fixture was fixed; all six verified-packaging tests then passed. No production logic was changed to accommodate the fixture.
- CLI clippy with tests and warnings denied passed. Monostyle passed.
- CLI help snapshots regenerated and reviewed: infra command list, verified packaging help, deploy evidence flags. Help snapshot test passed.
- No configuration schema types changed; schema regeneration is inapplicable.
- Podman 6.1.4 reported no socket and no machines. No real image build, extraction, runtime pull, registry push, production deployment, or container fixture ran.
- Only this clone's build outputs were reclaimed after local validation. Other agents' processes, sessions, caches, and the dirty main checkout were untouched.
- Still missing: real-engine committed fixture/pins and execution, exported RPC contract binding, and comprehensive fake-engine release/reuse/rollback/recovery fault injection. These remain acceptance gaps, not successful verification.

- Publication is blocked: the pre-push hook requires full-workspace clippy and other checks, beyond the authorized single heavy build. Free disk fell to 1.3 GiB amid other work; reclaiming this clone's target directory restored 7.4 GiB. The hook was not bypassed and no PR was created. Parent handoff is the signed local feature-branch commit; full pre-push checks and publication remain pending.

## Follow-up validation

The follow-up rebases on `23e5a1df`. The CLI library suite passed: 559 tests, 5 ignored. A tiny dependency-free locked backend fixture exports before a deliberate normal-startup panic. `verified-backend-acceptance.py` uses a signed synthetic commit of its committed tree and an invocation-only Git transport mapping to exercise full reproduction locally; this does not authenticate public GitHub provenance. It never publishes images or deploys.

RPC descriptors are still unsigned operator evidence. The independent descriptor pin authenticates the operator claim; no guest signature, approval, or runtime attestation is implied. Website and service evidence, and approval integration, remain outside this slice.
