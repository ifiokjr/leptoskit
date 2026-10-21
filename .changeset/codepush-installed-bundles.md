---
"release": minor
---

# Trial and roll back installed native bundles

`leptoskit_codepush::installed::InstalledBundles` owns the release a native shell runs. `sync_with` downloads and verifies an update and stores the signed container without running it. It also refuses bundles whose `runtime.json` names a different native ABI or shell contract, or a plugin the shell lacks. `launch` marks a downloaded release as a trial before returning its Wasm. `confirm` promotes it once the host has rendered it, and `reject` drops it and keeps the last working release. A trial left unconfirmed by process death is rejected on the next open. Confirmed releases load offline and have their signatures checked again on every launch. A newer store baseline supersedes older downloads.

The `http` feature adds a blocking `HttpTransport` for a serialized background worker. It requires HTTPS unless plaintext is explicitly allowed, follows no redirects, caps responses at 32 MiB, and gives each request 30 seconds. `CodePushClient::download_verified` authenticates an update without staging it or advancing the running release, and `bundle_directory_as` packs a store baseline with an explicit identity.
