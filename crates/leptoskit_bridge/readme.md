<h1 align="center">leptoskit_bridge</h1>

<p align="center">
  <a href="https://ifiokjr.github.io/leptoskit/"><img src="https://raw.githubusercontent.com/ifiokjr/leptoskit/main/docs/src/assets/mark.svg" alt="leptoskit" width="64"></a>
</p>

<p align="center">
  <strong>leptoskit_bridge</strong> is the bounded binary transport between a Wasm guest and the native plugins it is allowed to call.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://docs.rs/leptoskit_bridge"><strong>Docs</strong></a> ·
  <a href="https://github.com/ifiokjr/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<p align="center">
  <a href="https://crates.io/crates/leptoskit_bridge"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_bridge.svg"></a>
  <a href="https://docs.rs/leptoskit_bridge"><img alt="docs.rs" src="https://docs.rs/leptoskit_bridge/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<br>

## Overview

A native leptoskit app runs its interface as a Wasm guest, and the guest cannot touch the clipboard, the battery, or the filesystem itself. The host can. This crate is the channel between them: the guest submits a call to a named plugin method with a byte payload, the host does the native work later on its own thread, and the guest collects the reply. Native code can also push events to the guest.

The channel carries bytes and nothing else. There is no JSON envelope, no base64, no global registry, and no executor; each plugin defines its own payload format, usually with the small `codec` module here (little-endian integers, `u32`-length-prefixed strings, truncation and trailing-byte checks).

## Host side

The host owns one `Bridge` on its event-loop thread, registers plugins, and grants methods. Nothing is allowed until the host says so, and an over-the-air bundle cannot add native capabilities by naming a different ID.

A plugin written in Rust implements the `Plugin` trait (`name`, `methods`, `invoke`) for short, non-blocking work. Slower work uses the host's native async APIs and calls `Bridge::complete` when it finishes; `Bridge::cancel` and `Bridge::next_cancelled` tell the host which running work to stop.

## Guest side

In a Wasm bundle loaded by `leptoskit_wasm`, enable the `guest` feature (it exists only for `wasm32`) and use `leptoskit_bridge::guest`:

- `Client::new("name")` resolves a plugin once and caches its numeric ID;
- `Client::call(method, &payload)` returns a `Call` future. Dropping the future cancels the native request;
- `Events::take()` yields the single event receiver, with `recv` and `try_recv`.

The guest talks to the host through the `leptoskit_plugins_v1` import module and is woken by the exported `leptoskit_plugins_ready`. Most apps never use this directly; each `leptoskit_plugin_*` crate wraps it in a typed `guest` module.

## Limits

`Limits` bounds everything before it is accepted: outstanding calls (128 by default), buffered events (256), bytes per request, reply, or event (8 MiB), and total retained bytes (32 MiB). Exceeding one returns `Error::Busy` or `Error::TooLarge` instead of growing. Errors (`Unavailable`, `PermissionDenied`, `InvalidArgument`, `Busy`, `TooLarge`, `Cancelled`, `Native`, `InvalidCall`, `Exhausted`) have stable wire codes, shared with the Swift, Kotlin, and C++ hosts in [`leptoskit_bridge_ffi`](https://github.com/ifiokjr/leptoskit/tree/main/crates/leptoskit_bridge_ffi).

Payloads are `Bytes`, so Rust-to-Rust handoff moves ownership without copying; only crossing a Wasm linear-memory boundary copies. A `Bridge` is thread-confined and belongs to one application: drop it when the guest unloads, and late completions then return `InvalidCall`.

## Features

| Feature      | What it adds                                                                                                                                                                                                     |
| ------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `guest`      | The `guest` module for `wasm32` bundles run by the interpreter. Not for browser bundles.                                                                                                                         |
| `async-host` | `async_host::CancellationToken` and the `PluginFuture` handler type, so a native host can run cancellable async plugin work. Creates no runtime of its own.                                                      |
| `devtools`   | The `devtools` module: every call's timeline (submitted, running, answered or cancelled, read, answered late), sizes, and status, for leptoskit's devtools. Bounded at 256 records and allocation-free per call. |

The default build pulls in no async runtime, and CI checks that.

## Run and measure

```sh
cargo test -p leptoskit_bridge                      # unit, lifecycle, identity, allocation tests
cargo run --release -p leptoskit_bridge --example roundtrip   # devenv shell bench:bridge
```

The `roundtrip` example times submit, dequeue, completion, and consumption for payloads from 0 bytes to 1 MiB and asserts that no payload is copied. It excludes Wasm, JNI, scheduling, and OS work, so use it to watch the transport, not to compare frameworks.

## How it fits

Plugins define methods, payload limits, cancellation, and error codes once; the guest and the Swift, Kotlin, or C++ host then speak the same protocol. Most apps never name it directly, but every runtime plugin does.

## Use it

```rust
use leptoskit_bridge::Bridge;
use leptoskit_bridge::Bytes;
use leptoskit_bridge::MethodId;

let mut bridge = Bridge::default();
let echo = bridge.register("echo", [MethodId(0)])?;
bridge.grant(echo, MethodId(0), true)?;

// The guest (or a test) submits a call; the host drains and completes it.
let id = bridge.submit(echo, MethodId(0), Bytes::from_static(b"hello"))?;
let request = bridge.next_request().expect("a queued request");
bridge.complete(id, Ok(request.payload))?;
let reply = bridge.take_result(id)?.expect("ready")?;
assert_eq!(reply.as_ref(), b"hello");
# Ok::<(), leptoskit_bridge::Error>(())
```

## Learn more

- [Crate documentation](https://docs.rs/leptoskit_bridge)
- [leptoskit documentation](https://ifiokjr.github.io/leptoskit/)
- [Repository](https://github.com/ifiokjr/leptoskit)
- [Plugins](https://ifiokjr.github.io/leptoskit/platforms/plugins.html)
