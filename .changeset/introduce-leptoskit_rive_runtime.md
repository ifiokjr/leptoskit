---
leptoskit_rive_runtime: breaking
---

# leptoskit_rive_runtime: the pinned Rive runtime

`leptoskit_rive_runtime` is the pinned, offline C++ dependency used by `leptoskit_rive` native playback.

Applications use `leptoskit_rive`; this crate exposes no application API. Its separate Cargo build lets changes to the Rust player or image adapter reuse the C++ compilation cache.
