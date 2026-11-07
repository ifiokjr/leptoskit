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

The real-engine fixture is `crates/leptoskit_cli/tests/fixtures/verified-backend`. Run `devenv shell -- python3 docs/agents/verified-backend-acceptance.py <CLI>` after committing it, with `CONTAINER_CONNECTION` selecting a disposable local Podman VM. The script prints the builder/runtime digests and descriptor, independently hashes the extracted image executable, and checks rejection without success evidence. It exercises a local committed source with a process-local Git transport mapping, not public provenance authentication.

## Follow-up validation

The follow-up rebases on `23e5a1df`. The CLI library suite passed: 559 tests, 5 ignored. A tiny dependency-free locked backend fixture exports before a deliberate normal-startup panic. `verified-backend-acceptance.py` uses a signed synthetic commit of its committed tree and an invocation-only Git transport mapping to exercise full reproduction locally; this does not authenticate public GitHub provenance. It never publishes images or deploys.

RPC descriptors are still unsigned operator evidence. The independent descriptor pin authenticates the operator claim; no guest signature, approval, or runtime attestation is implied. Website and service evidence, and approval integration, remain outside this slice.

Review found the initial packaging implementation reused a text checksum for executable bytes. The follow-up uses raw-byte SHA-256 and tests invalid-UTF-8 substitutions. It also normalizes Podman image IDs and validates a descriptor before writing it.
