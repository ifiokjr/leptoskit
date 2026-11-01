<h1 align="center">leptoskit_plugin_crypto</h1>

<p align="center">
  <strong>leptoskit_plugin_crypto</strong> is the leptoskit plugin for portable SHA-256, SHA-512, secure random bytes, and UUID v4 with a binary native bridge.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://docs.rs/leptoskit_plugin_crypto"><strong>Docs</strong></a> ·
  <a href="https://github.com/ifiokjr/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<p align="center">
  <a href="https://crates.io/crates/leptoskit_plugin_crypto"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_plugin_crypto.svg"></a>
  <a href="https://docs.rs/leptoskit_plugin_crypto"><img alt="docs.rs" src="https://docs.rs/leptoskit_plugin_crypto/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<br>

## Overview

SHA-256, SHA-512, secure random bytes and UUID v4 with a binary native bridge. The digest APIs and method IDs remain unchanged. Native registration uses the Rust worker so hashing and operating-system entropy requests stay off the UI thread.

Native calls use [`getrandom`](https://docs.rs/getrandom/latest/getrandom/) and the operating system's cryptographically secure random source. Browser applications enable `web` to use `Crypto.getRandomValues`. Interpreter guests enable `guest` and use `guest::Crypto::{new, sha256, sha512, random_bytes, uuid_v4}` to call the native implementation. Browser support is opt-in so guest bundles do not import JavaScript. Direct hashing works on all targets without either feature. Direct randomness on a WASM target without the browser backend returns `Unavailable`; interpreter guests should use the native guest client.

`random_bytes` returns an owned binary buffer; `fill_random` fills an existing buffer without allocating an output buffer. Both reject requests over 65,536 bytes before accessing the entropy source. Entropy failures return an error; there is no pseudo-random or deterministic fallback. If `fill_random` fails, its buffer may be partially filled and must not be consumed as random data.

`UuidV4` stores the 16 canonical bytes and validates the version and variant when decoding. Its display representation uses lowercase, hyphenated UUID text. UUIDs use 122 random bits plus the version and variant required by [RFC 9562](https://www.rfc-editor.org/rfc/rfc9562.html#name-uuid-version-4). The guest client validates digest widths, random response lengths and UUID bits.

These APIs cover the digest, random-byte and UUID capabilities of [Expo Crypto](https://docs.expo.dev/versions/latest/sdk/crypto/). They do not expose encryption, key storage or additional digest algorithms.

| Method       | ID | Request                          | Response                   |
| ------------ | -- | -------------------------------- | -------------------------- |
| SHA-256      | 0  | Raw input                        | Exactly 32 bytes           |
| SHA-512      | 1  | Raw input                        | Exactly 64 bytes           |
| Random bytes | 2  | One u32 little-endian byte count | Exactly that many bytes    |
| UUID v4      | 3  | Empty                            | Exactly 16 canonical bytes |

Random-byte and UUID request decoders reject truncated or trailing data. All outputs remain binary; callers choose text encoding when their protocol needs it.

## How it fits

Add `leptoskit_plugin_crypto` as a direct dependency and select the `crypto` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Native hashing and entropy run on the Rust worker, and browsers use the same Rust implementation compiled to Wasm.

## Use it

```rust
use leptoskit_plugin_crypto::{random_bytes, sha256, uuid_v4};

# fn example() -> Result<(), leptoskit_bridge::Error> {
let nonce = random_bytes(32)?;
let digest = sha256(&nonce);
let id = uuid_v4()?;
assert_eq!(digest.len(), 32);
assert_eq!(id.to_string().len(), 36);
# Ok(())
# }
```

## Learn more

- [Crate documentation](https://docs.rs/leptoskit_plugin_crypto)
- [leptoskit documentation](https://ifiokjr.github.io/leptoskit/)
- [Repository](https://github.com/ifiokjr/leptoskit)
- [Plugin inventory](https://ifiokjr.github.io/leptoskit/platforms/plugin-inventory.html)
