<h1 align="center">leptoskit_plugin_store</h1>

<p align="center">
  <strong>leptoskit_plugin_store</strong> is the leptoskit plugin for bounded JSON application settings with explicit persistence and atomic ordered batches.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://docs.rs/leptoskit_plugin_store"><strong>Docs</strong></a> ·
  <a href="https://github.com/ifiokjr/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<p align="center">
  <a href="https://crates.io/crates/leptoskit_plugin_store"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_plugin_store.svg"></a>
  <a href="https://docs.rs/leptoskit_plugin_store"><img alt="docs.rs" src="https://docs.rs/leptoskit_plugin_store/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<br>

## Overview

Application settings inspired by [Tauri Store](https://v2.tauri.app/plugin/store/): JSON values, explicit save/load, and atomic ordered batches. Each application gets one store; guests cannot select filesystem paths.

Select `"store"` in `native.plugins` and depend on this crate in the native host. Cargo metadata registers `from_app_dirs` with `rust-worker = true`. The factory only captures the app data directory; directory creation, initial loading, and all later disk I/O happen on the registered Rust worker.

`set`, `remove`, and `clear` stage changes in memory. `save` writes all staged entries once; there is no autosave timer or implicit save on drop/shutdown. `load` reloads the saved document and discards staged edits only after the full document passes validation. A missing document loads as an empty store.

`apply(changes, Save::Commit)` validates the entire ordered batch, saves it with one atomic replacement, then publishes it in memory. A failed commit leaves both the previous in-memory state and saved document intact. A failed `save` keeps staged edits available for retry. JSON `null` is distinct from a missing key. Keys and listings have deterministic lexical order.

Native storage uses `<app data>/store/settings.json` through an open directory capability and reuses the filesystem plugin's atomic replacement implementation. Symlinks cannot escape the capability. Files are fully written and synced before rename; a successful return does not promise directory-entry durability during sudden power loss. Malformed or oversized saved data is reported as an error, never silently overwritten or replaced with an empty store.

The browser `web::Store` uses the origin's real `localStorage`, storing the entire document under `leptoskit:store:v1` with one atomic `setItem`. Operations are synchronous and can block the browser main thread, so use this for small settings. Quota failures map to `TooLarge`; security-policy failures to `PermissionDenied`; missing storage support to `Unavailable`; other failures to `Native`. Browser storage can be cleared or evicted by the browser/user and is scoped to the origin.

Instances and tabs have independent staged state: the last successful save wins. Call `load` explicitly to observe external edits; there is no cross-process transaction isolation or change-notification stream. Applications needing those properties should use a database. **This store is not encrypted.** Use the secure store plugin for credentials.

Bounds apply before changes are published:

- 256 UTF-8 bytes per nonempty key; control characters are rejected.
- 64 KiB serialized JSON per value and 64 nesting levels.
- 1,024 keys and 512 KiB for the complete versioned document.
- 256 changes and 512 KiB per binary batch envelope.

The wire protocol uses little-endian `u32` and length-prefixed UTF-8/JSON bytes. Method `0` gets one key; `1` returns the current versioned document; `2` applies a batch; `3` saves; `4` reloads. Batch input is save mode (`0` deferred, `1` commit), change count, then tagged changes (`0` set with key/value, `1` remove with key, `2` clear). Get replies carry a presence flag and a JSON value only when present. All decoders reject malformed and trailing input.

## How it fits

Add `leptoskit_plugin_store` as a direct dependency and select the `store` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::Store` rather than resolving the plugin on every call. Each application gets one store, values are JSON over a binary envelope, and guests cannot choose filesystem paths.

## Use it

```rust,ignore
use leptoskit_plugin_store::{Change, Save, Value};
let store = leptoskit_plugin_store::guest::Store::new()?;
store.apply(&[
    Change::Set("theme".into(), Value::from("dark")),
    Change::Set("launches".into(), Value::from(7)),
], Save::Commit).await?;
```

## Learn more

- [Crate documentation](https://docs.rs/leptoskit_plugin_store)
- [leptoskit documentation](https://ifiokjr.github.io/leptoskit/)
- [Repository](https://github.com/ifiokjr/leptoskit)
- [Plugin inventory](https://ifiokjr.github.io/leptoskit/platforms/plugin-inventory.html)
