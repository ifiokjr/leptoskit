---
"release": minor
---

# Pull the newest code push release from a bucket

`leptoskit codepush pull` copies a channel's newest release from an S3-compatible bucket into the release store, so a CI runner that starts empty can number the next build and compute its delta from the release devices run. Before anything is written, the release has to be signed by a key in `codepush.public_keys` and its container has to match the signed manifest. `pull_channel` and `ReleaseStore::import_release` expose the same thing to Rust callers.
