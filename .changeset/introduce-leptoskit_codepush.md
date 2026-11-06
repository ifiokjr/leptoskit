---
leptoskit_codepush: major
---

# leptoskit_codepush: over-the-air updates

`leptoskit_codepush` ships interface changes to native apps without a store review: signed, content-hashed releases of the Wasm bundle, packed into one file, with binary deltas so a device downloads only the chunks that changed.

`leptoskit codepush` packs, signs, and publishes releases to a static channel layout that any file server, object store, or CDN can host, and `leptoskit_wasm` stages them on the device. A client accepts only releases signed by a trusted key and newer than what it runs, so a compromised registry can neither ship its own code nor roll a device back. A host can also require independent source approvals: rebuilders who built the same files from a public commit and signed to say so.
