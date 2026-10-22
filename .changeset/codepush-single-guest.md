---
"release": patch
---

# Ship the mobile guest once and launch it with one read

Generated iOS and Android apps now bundle the guest only inside its store container, `app.lkb`, instead of shipping it a second time as `app.wasm`. With code push off, or if the update store fails, the host reads the Wasm through the new `lk_bundle_entry` FFI call, which wraps `installed::entry_module` and checks every file hash. Mobile assets now live in `.leptoskit/mobile`, and Android's Gradle template points there. Desktop hosts keep `.leptoskit/assets/app.wasm`.

`InstalledBundles::launch` now reads, verifies, and decompresses only the release that runs, instead of every stored release on each launch. A release damaged since `open` is skipped: a damaged candidate is rejected, and a damaged confirmed release falls back to the store baseline.
