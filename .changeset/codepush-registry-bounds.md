---
"release": patch
---

Bound containers and total expansion to 32 MiB, manifests to 1 MiB, and bundles to 4096 files. Reject over-limit container writes and release creation before changing existing files or the release store. S3 reads now enforce separate response limits for indexes (2 MiB), signatures (4 KiB), and releases or deltas (32 MiB), including responses without a content length. Omit over-limit optional deltas from publication so interrupted publishes can resume and devices download the full release.
