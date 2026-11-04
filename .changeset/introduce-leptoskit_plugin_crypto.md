---
leptoskit_plugin_crypto: major
---

# leptoskit_plugin_crypto: portable digests and randomness

`leptoskit_plugin_crypto` is the leptoskit plugin for portable SHA-256, SHA-512, secure random bytes, and UUID v4 with a binary native bridge.

Add `leptoskit_plugin_crypto` as a direct dependency and select the `crypto` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic and hashing and entropy run on the host's Rust worker. Guests enable the `guest` feature and call the host through `guest::Crypto`. Hashing works directly on every target; browser apps enable the `web` feature for random bytes and UUIDs from `Crypto.getRandomValues`. There is no pseudo-random fallback, and encryption and key storage are out of scope.
