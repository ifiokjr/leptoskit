# leptoskit_codepush

> Code push for wasm app bundles: the React Native over-the-air update model.

A release is a `BundleManifest` — content-hashed, gzip-compressed files packed into a single `.lkb` container. Clients check in with their current version and receive either an up-to-date verdict or a download plan: the full bundle, or a binary delta against exactly what they already run. Every download is hash-verified before it stages.

- `create_delta` / `apply_delta` — block-indexed binary deltas that shrink a typical wasm update to a fraction of the full download;
- `ReleaseStore` — the on-disk registry, computing deltas against the previous release on ingest;
- `codepush_router` (feature `server`) — the Axum check/download/delta routes;
- `CodePushClient` — the updater, transport-agnostic so each platform shell brings its own HTTP stack.
