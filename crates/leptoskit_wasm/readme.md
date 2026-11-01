<h1 align="center">leptoskit_wasm</h1>

<p align="center">
  <a href="https://ifiokjr.github.io/leptoskit/"><img src="https://raw.githubusercontent.com/ifiokjr/leptoskit/main/docs/src/assets/mark.svg" alt="leptoskit" width="64"></a>
</p>

<p align="center">
  <strong>leptoskit_wasm</strong> runs the app's interface bundle in `wasmi`, drives a native backend through a versioned, fuel-metered ABI, and applies code push releases.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://docs.rs/leptoskit_wasm"><strong>Docs</strong></a> ·
  <a href="https://github.com/ifiokjr/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<p align="center">
  <a href="https://crates.io/crates/leptoskit_wasm"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_wasm.svg"></a>
  <a href="https://docs.rs/leptoskit_wasm"><img alt="docs.rs" src="https://docs.rs/leptoskit_wasm/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<br>

## Overview

The app shipped through the store is a thin platform shell; the _interface_ is a wasm module fetched over the air. This crate runs that module in [`wasmi`](https://crates.io/crates/wasmi) — a pure interpreter, which is legal on iOS where JIT compilation is not — and drives a `leptoskit_native::NativeSurface` through a small, versioned ABI.

## The ABI

The module imports from the `leptoskit` module. Ids are `i64`s the module chooses; pointers, lengths, and indices are `i32`s read as unsigned.

| Function                            | Meaning                                 |
| ----------------------------------- | --------------------------------------- |
| `node_create(id, kind)`             | create a widget                         |
| `node_set_props(id, ptr, len)`      | set props from JSON in wasm memory      |
| `node_insert(parent, child, index)` | attach a child                          |
| `node_remove(parent, child)`        | detach a child                          |
| `node_delete(id)`                   | destroy a widget                        |
| `commit()`                          | hand the assembled tree to the renderer |

The module exports:

| Export                                | Required           | Meaning                                                                  |
| ------------------------------------- | ------------------ | ------------------------------------------------------------------------ |
| `memory`                              | yes                | the module's linear memory                                               |
| `leptoskit_main()`                    | yes                | build and commit the initial tree                                        |
| `leptoskit_event(id, kind, ptr, len)` | yes                | rebuild after an event; returns `1` when handled                         |
| `leptoskit_alloc(len) -> ptr`         | for payload events | returns a non-zero pointer to `len` writable bytes for the event payload |
| `leptoskit_abi_version() -> i32`      | no                 | checked against `ABI_VERSION` on load; a mismatch is refused             |

Every entry point is fuel-metered, so a bad bundle cannot hang the device (it fails with `WasmError::OutOfFuel`), and every commit is diffed by the renderer, so a rebuild is cheap.

Each host permits one instance, one linear memory of at most 64 MiB (`MAX_MEMORY_BYTES`), and two tables with at most 65,536 elements each (`MAX_TABLE_ELEMENTS`). This accommodates Rust's indirect-function and externref tables while keeping the total bounded. Limits cover unexported resources, initial allocations, the module's start function, and all later callbacks. Oversized allocations, excess resource counts, and growth beyond these budgets return `WasmError::ResourceLimitExceeded`. Growth within the host budget retains normal WebAssembly failure behavior, including `-1` for guest-declared maxima and `WasmError::OutOfFuel` for exhausted fuel. These budgets cover guest linear memory and tables; interpreter metadata and host-side allocations are separate.

Widget kind codes are append-only: `View` through `Button` retain codes 0–6, followed by `Switch` (7), `Checkbox` (8), `Radio` (9), `Slider` (10), `LinearProgressIndicator` (11), `ActivityIndicator` (12), `PasswordInput` (13), `Pressable` (14), and `Vector` (15). Existing event codes 0–5 retain their meaning; `Toggle` is 6 with `{"checked":true}`, and `ValueChange` is 7 with `{"value":0.5}`. These payloads carry JSON booleans and numbers. Slider configuration uses `{"slider":{"min":0,"max":1,"value":0.5,"step":0.25}}`; progress uses `{"progress":{"determinate":0.5}}` or `{"progress":"indeterminate"}`. A vector's paths travel as `{"drawing":{"view_box":{"min_x":0,"min_y":0,"width":24,"height":24},"scaling":"fit","paths":[{"data":"M 4 12 L 20 12","stroke":{"color":4294967295,"width":2,"cap":"round","join":"round"}}]}}`, with path data in canonical form: absolute `M`, `L`, `Q`, `C`, and `Z` commands, every token separated by one space. A host older than code 15 draws a vector as an empty `View`.

`WasmHost::dispatch` enforces visibility, enabled state, control kinds, and finite slider proposals, then snaps and clamps slider values. Controlled state changes only when the guest commits updated properties. `dispatch_wire` is a lower-level guest entry point and leaves payload interpretation to the guest.

### Tree rules

The host keeps the wire tree a tree and traps the offending call otherwise:

- `node_insert` moves a child that already has a parent, like the DOM's `insertBefore`; an `index` past the end appends.
- `node_insert` refuses to put a node inside itself or its own descendants, and the tree may be at most `MAX_TREE_DEPTH` (256) levels deep.
- `node_delete` destroys one node; its children become parentless and must be reattached or deleted.
- `commit` renders the single parentless node as the root. More than one parentless node traps; no nodes at all leaves the previous interface on screen.
- Control values are validated when decoded, and complete trees are validated before commit. Leaf children, missing radio groups, duplicate radio selection, and invalid slider/progress values trap before changing the committed native interface.

### Memory ownership

The module owns its memory. `node_set_props` copies the bytes during the call, so the module may reuse them immediately. Event payloads are written into a buffer from `leptoskit_alloc`; the host never frees it, so the module reclaims it once `leptoskit_event` returns.

Because updates are just new wasm bundles on a channel, shipping a fix works exactly like React Native's code push: no store review, instant rollback, and the same `leptoskit_codepush` manifests and deltas as every other artifact.

## How it fits

This is what makes the shipped app a shell: the interface arrives as Wasm, renders through the same renderer, and can be replaced without a store update. The generated iOS, Android, macOS, Windows, and Linux hosts embed it.

## Use it

```toml
# Generated native hosts embed this crate; app code never calls it directly.
[dependencies]
leptoskit_wasm = "0.1"
```

## Learn more

- [Crate documentation](https://docs.rs/leptoskit_wasm)
- [leptoskit documentation](https://ifiokjr.github.io/leptoskit/)
- [Repository](https://github.com/ifiokjr/leptoskit)
- [Code push](https://ifiokjr.github.io/leptoskit/guide/05-code-push.html)
