<h1 align="center">leptoskit_plugin_secure_store</h1>

<p align="center">
  <strong>leptoskit_plugin_secure_store</strong> is the leptoskit plugin for small binary secrets in the platform credential store, scoped to one application.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://docs.rs/leptoskit_plugin_secure_store"><strong>Docs</strong></a> ·
  <a href="https://github.com/ifiokjr/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<p align="center">
  <a href="https://crates.io/crates/leptoskit_plugin_secure_store"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_plugin_secure_store.svg"></a>
  <a href="https://docs.rs/leptoskit_plugin_secure_store"><img alt="docs.rs" src="https://docs.rs/leptoskit_plugin_secure_store/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<br>

## Overview

Small binary secrets in native credential stores. Each app explicitly grants the `secure-store` plugin; guest calls then use a cached numeric plugin ID. Add this crate as a direct dependency and include `secure-store` in `native.plugins` for automatic iOS/Android registration.

A service and key each contain 1–128 ASCII letters, digits, dots, underscores or hyphens. A value contains at most 2048 bytes, including an empty value. Requests with malformed framing, unsupported methods, invalid names, trailing data or oversized values fail before storage access. No key enumeration or bulk deletion is provided. Applications should retain the keys they own and explicitly delete credentials when signing out.

All newly written secrets share one app-owned encrypted record, limited to **2560 encoded bytes and 64 entries**. Names, framing, and empty values count toward those budgets. This matches Windows Credential Manager's credential blob limit. Quota failures return `TooLarge` before persistent mutation; the previous record remains intact. Trusted hosts can lower these limits with Rust `SecureStore::with_quota(application, Quota { max_bytes, max_entries })`, Swift `SecureStorePlugin(quota: SecureStoreQuota(maxBytes:maxEntries:))`, or Android constructor arguments `maxBytes` and `maxEntries`. Overwrite and deletion release capacity. The encrypted record is the usage source of truth across restarts; process and file locks serialize updates. Keep these host-owned lock files in place while the application is running. Atomic writes need one additional record of temporary headroom.

Upgrades retain read/delete access to old per-key credentials. Rewriting a key moves its value into the bounded record and deletes the old copy. Old credentials are never created or expanded by this version, but pre-existing legacy storage is outside the new quota until explicitly migrated or deleted. Hosts upgrading a previously unbounded store should clean up its known keys. Corruption of the shared encrypted record fails closed; a trusted host must repair or remove that record before it can be used again.

## Platform behavior

| Platform | Storage and lifecycle                                                                                                                                                                                                                                                                                                                                                                  |
| -------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| iOS      | One generic-password Keychain item, scoped by bundle identifier. Service/key identities live inside the encrypted record. `WhenUnlockedThisDeviceOnly`, not synchronized, no shared access group. Reads while locked fail. Items can survive uninstall/reinstall with the same signing identity; never depend on either survival or automatic deletion.                                |
| Android  | A non-exportable Android Keystore AES-256 key encrypts the record with AES-GCM and a fresh platform-generated IV per write. App identity is authenticated as additional data; service/key identities are inside the authenticated ciphertext. Only ciphertext is stored in an atomic file under `Context.noBackupFilesDir`. Uninstall clears app files and the app's Keystore entries. |
| macOS    | Rust `keyring` backend uses the user's Keychain. The supplied app identifier scopes service names; OS Keychain access policy applies. The Swift handler also supports macOS and uses the accessibility policy described for iOS.                                                                                                                                                       |
| Windows  | Rust `keyring` backend uses Windows Credential Manager.                                                                                                                                                                                                                                                                                                                                |
| Linux    | Rust `keyring` backend uses the desktop Secret Service over D-Bus with encrypted session transport. A running, accessible service is required; no plaintext-file or process-memory fallback is configured.                                                                                                                                                                             |
| Web      | Unavailable. `web::available()` is false; all storage calls return `Error::Unavailable`. Browser localStorage and IndexedDB are not treated as OS credential stores.                                                                                                                                                                                                                   |

Desktop hosts create `SecureStore::new("com.example.app")` and register it with the runtime. Credential-store operations may block and should execute off the UI thread. Swift Keychain IPC and Android file/Keystore work run on background executors. Store operations serialize across instances and processes belonging to the same app.

Android does not require biometric authentication or a fresh device unlock for each operation. Hardware backing depends on the device's Keystore implementation and is not assumed. Apple and Android handlers queue concurrent requests; cancellation cannot interrupt already-started credential IPC or a file lock acquisition.

Missing entries return `None`. Corrupt ciphertext, missing encryption keys, invalidated keys, locked stores, and storage failures return errors. The plugin does not silently reset a failed store or overwrite an unreadable record. If an Android record's encryption key is lost, trusted host code must remove the undecryptable ciphertext before the plugin can create a fresh store. Canceling a request does not guarantee rollback of an OS write that has already started.

The application namespace on desktop is not a security sandbox: the logged-in user's credential service defines access. Small secrets still enter guest and native memory while being used; this API does not promise zeroization of every copy. Android clears its temporary plaintext arrays after use. It never writes plaintext to persistent storage or logs secret values.

## Scope and wire format

This implements get/set/delete for small secrets, inspired by [Expo SecureStore](https://docs.expo.dev/versions/latest/sdk/securestore/). It does not yet expose Expo's biometric authentication, access groups or configurable accessibility policies. It is not a port of [Tauri Stronghold](https://v2.tauri.app/plugin/stronghold/), which has a distinct vault/snapshot model and cryptographic operations.

All lengths and flags are unsigned little-endian 32-bit integers. A request is `service_length | service_utf8 | key_length | key_utf8`; set appends `value_length | value_bytes`. Methods are get `0`, set `1`, delete `2`. Get replies are a presence flag (`0` or `1`) followed by raw bytes only when present. Set and delete reply with an empty buffer. The Rust decoder borrows the input; no JSON or base64 transformation is used on the bridge.

References: [Android Keystore](https://developer.android.com/privacy-and-security/keystore), [Android non-backup files](https://developer.android.com/reference/android/content/Context#getNoBackupFilesDir()), [keyring Rust platform backends](https://docs.rs/keyring/3.6.3/keyring/).

## How it fits

Add `leptoskit_plugin_secure_store` as a direct dependency and select the `secure-store` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::SecureStore` rather than resolving the plugin on every call. Native mobile handlers ship with the crate; desktop hosts construct the store over their own credential backend, and secrets never become a guest-selected filesystem path.

## Use it

```rust,ignore
use leptoskit_plugin_secure_store::{Key, guest::SecureStore};

let store = SecureStore::new()?;
let key = Key::new("account", "refresh-token")?;
store.set(key, token_bytes).await?;
let token = store.get(key).await?; // None is different from an empty byte string.
store.delete(key).await?;         // Deleting a missing entry succeeds.
```

## Learn more

- [Crate documentation](https://docs.rs/leptoskit_plugin_secure_store)
- [leptoskit documentation](https://ifiokjr.github.io/leptoskit/)
- [Repository](https://github.com/ifiokjr/leptoskit)
- [Plugin inventory](https://ifiokjr.github.io/leptoskit/platforms/plugin-inventory.html)
