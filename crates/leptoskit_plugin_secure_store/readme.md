# leptoskit_plugin_secure_store

Small binary secrets in native credential stores. Each app explicitly grants the `secure-store` plugin; guest calls then use a cached numeric plugin ID. Add this crate as a direct dependency and include `secure-store` in `native.plugins` for automatic iOS/Android registration.

```rust,ignore
use leptoskit_plugin_secure_store::{Key, guest::SecureStore};

let store = SecureStore::new()?;
let key = Key::new("account", "refresh-token")?;
store.set(key, token_bytes).await?;
let token = store.get(key).await?; // None is different from an empty byte string.
store.delete(key).await?;         // Deleting a missing entry succeeds.
```

A service and key each contain 1–128 ASCII letters, digits, dots, underscores or hyphens. A value contains at most 2048 bytes, including an empty value. Requests with malformed framing, unsupported methods, invalid names, trailing data or oversized values fail before storage access. No key enumeration or bulk deletion is provided. Applications should retain the keys they own and explicitly delete credentials when signing out.

## Platform behavior

| Platform | Storage and lifecycle                                                                                                                                                                                                                                                                                                                                                                                                                                    |
| -------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| iOS      | Generic-password Keychain items, scoped by bundle identifier + service + key. `WhenUnlockedThisDeviceOnly`, not synchronized, no shared access group. Reads while locked fail. Items can survive uninstall/reinstall with the same signing identity; never depend on either survival or automatic deletion.                                                                                                                                              |
| Android  | A non-exportable Android Keystore AES-256 key encrypts values with AES-GCM. A fresh platform-generated IV is used for each write. The full app/service/key identity is authenticated as additional data, so swapping entries fails authentication. Only ciphertext is stored in atomic files under `Context.noBackupFilesDir`; Android backup cannot restore ciphertext without its lost key. Uninstall clears app files and the app's Keystore entries. |
| macOS    | Rust `keyring` backend uses the user's Keychain. The supplied app identifier scopes service names; OS Keychain access policy applies. The Swift handler also supports macOS and uses the accessibility policy described for iOS.                                                                                                                                                                                                                         |
| Windows  | Rust `keyring` backend uses Windows Credential Manager.                                                                                                                                                                                                                                                                                                                                                                                                  |
| Linux    | Rust `keyring` backend uses the desktop Secret Service over D-Bus with encrypted session transport. A running, accessible service is required; no plaintext-file or process-memory fallback is configured.                                                                                                                                                                                                                                               |
| Web      | Unavailable. `web::available()` is false; all storage calls return `Error::Unavailable`. Browser localStorage and IndexedDB are not treated as OS credential stores.                                                                                                                                                                                                                                                                                     |

Desktop hosts create `SecureStore::new("com.example.app")` and register it with the runtime. Credential-store operations may block and should execute off the UI thread. Swift Keychain IPC and Android file/Keystore work run on background executors. Android serializes operations within the application process; multiple processes accessing this store are not supported. Swift and desktop writes are individual OS operations; no cross-entry transaction is promised.

Android does not require biometric authentication or a fresh device unlock for each operation. Hardware backing depends on the device's Keystore implementation and is not assumed. Apple handlers permit one outstanding Keychain operation and return `Busy` for concurrent requests; cancellation keeps that slot occupied until already-started IPC finishes. Android waiting operations use a cancellable mutex.

Missing entries return `None`. Corrupt ciphertext, missing encryption keys, invalidated keys, locked stores, and storage failures return errors. The plugin does not silently reset a failed store or overwrite an unreadable entry on a read. Deleting the remaining ciphertext entries permits a fresh Android key to be created on the next write if the previous key was lost. Canceling a request does not guarantee rollback of an OS write that has already started.

The application namespace on desktop is not a security sandbox: the logged-in user's credential service defines access. Small secrets still enter guest and native memory while being used; this API does not promise zeroization of every copy. Android clears its temporary plaintext arrays after use. It never writes plaintext to persistent storage or logs secret values.

## Scope and wire format

This implements get/set/delete for small secrets, inspired by [Expo SecureStore](https://docs.expo.dev/versions/latest/sdk/securestore/). It does not yet expose Expo's biometric authentication, access groups or configurable accessibility policies. It is not a port of [Tauri Stronghold](https://v2.tauri.app/plugin/stronghold/), which has a distinct vault/snapshot model and cryptographic operations.

All lengths and flags are unsigned little-endian 32-bit integers. A request is `service_length | service_utf8 | key_length | key_utf8`; set appends `value_length | value_bytes`. Methods are get `0`, set `1`, delete `2`. Get replies are a presence flag (`0` or `1`) followed by raw bytes only when present. Set and delete reply with an empty buffer. The Rust decoder borrows the input; no JSON or base64 transformation is used on the bridge.

References: [Android Keystore](https://developer.android.com/privacy-and-security/keystore), [Android non-backup files](https://developer.android.com/reference/android/content/Context#getNoBackupFilesDir()), [keyring Rust platform backends](https://docs.rs/keyring/3.6.3/keyring/).
