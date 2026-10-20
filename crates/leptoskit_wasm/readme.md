<p align="center">
  <a href="https://ifiokjr.github.io/leptoskit/"><img src="https://raw.githubusercontent.com/ifiokjr/leptoskit/main/docs/src/assets/mark.svg" alt="leptoskit" width="64"></a>
</p>

# leptoskit_wasm

> The on-device wasm runtime host: the code push runtime for native apps.

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

Widget kind codes are append-only: `View` through `Button` retain codes 0–6, followed by `Switch` (7), `Checkbox` (8), `Radio` (9), `Slider` (10), `LinearProgressIndicator` (11), `ActivityIndicator` (12), `PasswordInput` (13), and `Pressable` (14). Existing event codes 0–5 retain their meaning; `Toggle` is 6 with `{"checked":true}`, and `ValueChange` is 7 with `{"value":0.5}`. These payloads carry JSON booleans and numbers. Slider configuration uses `{"slider":{"min":0,"max":1,"value":0.5,"step":0.25}}`; progress uses `{"progress":{"determinate":0.5}}` or `{"progress":"indeterminate"}`.

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
