---
"release": minor
---

# Make code push deltas pay off and harden the update paths

Deltas used to be computed between gzip streams, which differ in every byte after the first change, and stored inserted bytes as a pretty-printed JSON array, so a delta was usually larger than the full download. Containers now compress each file in chunks of about 64 KiB that come out the same whenever their input does, so a delta carries only the chunks an update changed; inserted bytes travel as base64 in compact JSON; and `release` keeps a delta only when it is at most three quarters of the full container. Containers stay ordinary gzip, so older clients read them unchanged; a delta in the new encoding fails to parse on an older client, which falls back to the full download. Chunked compression makes a container up to a few percent larger than plain gzip.

A release that failed activation on a device is no longer downloaded again on every check: while the channel still names it, `InstalledBundles::sync` reads only `index.json` and reports the working release as current, instead of downloading up to 32 MiB and failing.

`CodePushClient::with_max_container_bytes` now bounds full downloads and delta bodies as well as delta output, and `CodePushClient::sync` replaces its staging directory whole, so files of the previous release no longer survive an update and a failed sync leaves the previous staging intact.

The release store assembles each release directory, container, signature, and delta, under a temporary name and renames it into place, so a crash can no longer leave an unsigned release that blocks re-releasing that version. `pull` checks the bucket's container against the signed manifest before decompressing anything, and containers with bytes after their last file, or after a file's gzip stream, are rejected. The code push routes log the cause of every `500` through `tracing`, while the client still sees only a fixed message.
