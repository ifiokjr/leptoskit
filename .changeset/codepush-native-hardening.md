---
"release": patch
---

# Harden native code push against hostile and failing states

A 404 from a static registry now means "no release yet" on mobile: `HttpTransport` returns `CodePushError::NotFound` for it, so a channel with no `index.json` reports up to date instead of failing. Requests get 15 seconds to connect and 30 seconds for a response. A whole download gets 10 minutes, so large bundles arrive over slow links.

`CodePushClient::with_max_container_bytes` caps what a delta may rebuild, and `InstalledBundles` sets it to 32 MiB, so a hostile delta can't allocate more than a full download would. `apply_delta_within` takes the cap directly.

A new store build supersedes every earlier download, even when the app version is unchanged. Only the working release and a staged one stay on disk. Interrupted writes no longer leave temporary files behind.

The generated iOS and Android hosts run the store-shipped guest when the update store can't open or launch, instead of showing an empty screen. A guest that builds its tree after an asynchronous step is no longer rejected for rendering nothing on its first commit.
