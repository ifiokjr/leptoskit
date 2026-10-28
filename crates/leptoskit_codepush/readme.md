<p align="center">
  <a href="https://ifiokjr.github.io/leptoskit/"><img src="https://raw.githubusercontent.com/ifiokjr/leptoskit/main/docs/src/assets/mark.svg" alt="leptoskit" width="64"></a>
</p>

# leptoskit_codepush

> Code push for wasm app bundles: the React Native over-the-air update model.

A release is a `BundleManifest` — content-hashed, gzip-compressed files packed into a single `.lkb` container. Releases live in a static layout: immutable files plus one `index.json` per channel naming the newest release, so any file server, object store, or CDN can host a channel with no server running. Clients read the index and fetch either the full bundle or a binary delta addressed by the hash of exactly what they already run. Every release is signed with the publisher's ed25519 key, and a client stages only releases that are signed by a key it trusts, meant for its app and channel, and newer than what it runs. Every download is hash-verified against that signed manifest before it stages.

- `create_delta` / `apply_delta` — block-indexed binary deltas between containers. Containers compress each file in chunks of about 64 KiB that come out the same whenever their input does, so an update that changes part of a module ships as the chunks it touched;
- `ReleaseStore` — the on-disk registry in the static layout, computing a delta against the previous release on ingest and keeping it when it is at most three quarters of the full container;
- `publish_channel` / `pull_channel` / `Bucket` — upload a channel to an object store, `index.json` last, and seed a store from one after verifying the newest release;
- `codepush_router` (feature `server`) — Axum routes serving the same static files, plus the older check/download/delta endpoints;
- `ReleaseSigningKey` / `TrustedKeys` — publisher signing and the key set an app shell pins, with room to rotate;
- `CodePushClient` — the updater, transport-agnostic so each platform shell brings its own HTTP stack.
