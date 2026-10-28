---
"release": patch
---

# Bound the cost of every code push request

The code push server's dynamic `check`, `download`, and `delta` routes answered from a listing of the channel, so every request read, decompressed, and hash-checked every release on it. They now answer from `index.json` and at most two more files of the static layout, as the static routes always have: a request costs the same on a channel with a thousand releases as on one with two, and a flood of update checks can no longer be turned into disk and CPU work. A release whose files are on disk but which `index.json` does not name yet is not offered, which is the guarantee the static layout already made.

Storage failures in the code push routes and in `leptoskit_axum`'s SPA fallback are a `500` with a fixed message. Filesystem paths and file contents no longer reach the client.
