<p align="center">
  <a href="https://ifiokjr.github.io/leptoskit/"><img src="https://raw.githubusercontent.com/ifiokjr/leptoskit/main/docs/src/assets/mark.svg" alt="leptoskit" width="64"></a>
</p>

# leptoskit_codepush

> Code push for wasm app bundles: the React Native over-the-air update model.

A release is a `BundleManifest` — content-hashed, gzip-compressed files packed into a single `.lkb` container. Clients check in with their current version and receive either an up-to-date verdict or a download plan: the full bundle, or a binary delta against exactly what they already run. Every release is signed with the publisher's ed25519 key, and a client stages only releases that are signed by a key it trusts, meant for its app and channel, and newer than what it runs. Every download is hash-verified against that signed manifest before it stages.

- `create_delta` / `apply_delta` — block-indexed binary deltas that shrink a typical wasm update to a fraction of the full download;
- `ReleaseStore` — the on-disk registry, computing deltas against the previous release on ingest;
- `codepush_router` (feature `server`) — the Axum check/download/delta routes;
- `ReleaseSigningKey` / `TrustedKeys` — publisher signing and the key set an app shell pins, with room to rotate;
- `CodePushClient` — the updater, transport-agnostic so each platform shell brings its own HTTP stack.
