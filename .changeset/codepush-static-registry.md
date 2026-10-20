---
"release": minor
---

# Serve code push from any static host

Releases now live in a static layout that any object store or CDN can serve with no server running. Each channel has one `index.json`, which holds the newest release's signed manifest. Every release file is immutable, and deltas are addressed by the SHA-256 of the container they apply to. `CodePushClient` reads the index, verifies it, and asks for the delta from its exact container. A 404 sends it to the full download. `leptoskit codepush serve` serves the same files, and the older `check`, `download`, and `delta` endpoints keep working.

`leptoskit codepush publish` uploads a channel to an S3-compatible bucket (Backblaze B2, Cloudflare R2, AWS S3, MinIO), writing `index.json` last so devices never see a release before its files. Credentials come only from `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`, and `AWS_SESSION_TOKEN`. Publishing refuses to roll a channel back or replace a release the bucket already holds with different contents.

Breaking: `ReleaseStore::ingest_directory` returns an `IngestedRelease`, `ReleaseRecord` no longer carries `delta_from_previous` (use `ReleaseStore::delta`), and a `Transport` must report a 404 as `CodePushError::NotFound`.
