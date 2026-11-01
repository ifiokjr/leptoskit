---
leptoskit_plugin_crypto: breaking
---

# leptoskit_plugin_crypto: portable digests and randomness

`leptoskit_plugin_crypto` is the leptoskit plugin for portable SHA-256, SHA-512, secure random bytes, and UUID v4 with a binary native bridge.

Add `leptoskit_plugin_crypto` as a direct dependency and select the `crypto` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Native hashing and entropy run on the Rust worker, and browsers use the same Rust implementation compiled to Wasm.
