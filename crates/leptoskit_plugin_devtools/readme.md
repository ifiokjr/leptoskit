<h1 align="center">leptoskit_plugin_devtools</h1>

<p align="center">
  <strong>leptoskit_plugin_devtools</strong> is the bridge plugin that carries a native guest's DevTools data to the host that already reports the tree.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://docs.rs/leptoskit_plugin_devtools"><strong>Docs</strong></a> ·
  <a href="https://github.com/ifiokjr/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<p align="center">
  <a href="https://crates.io/crates/leptoskit_plugin_devtools"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_plugin_devtools.svg"></a>
  <a href="https://docs.rs/leptoskit_plugin_devtools"><img alt="docs.rs" src="https://docs.rs/leptoskit_plugin_devtools/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<br>

## Overview

The bridge plugin that carries a native guest's DevTools data to its host. A native app's host reports the element tree to the DevTools hub itself, but only the guest knows its route, queries, forms, and named state. The guest's agent in `leptoskit_devtools` sends those through this plugin, the host merges them into the session it already reports, and commands for the guest (`SetState`, `Navigate`, `InvalidateQuery`, `RefetchQuery`, `SetQueryData`, and `RequestSnapshot`) come back the same way.

Apps never select it. The shared app runtime installs it when `leptoskit dev` builds the host with `leptoskit_bridge_ffi/devtools`, and a guest on any other host finds no plugin and reports nothing.

Wire methods: `0` takes a `Hello` (the protocol version and what the guest can feed); `1` takes a `Report` (values that changed, or with `reset` every value the guest holds); `2` waits until the hub sends commands for the guest and returns up to 64 of them; `3` takes a little-endian `u32` of milliseconds, at most 1000, and answers once that long has passed on the host's clock, which is how a guest without timers waits out its debounce without asking for display frames. The first three carry JSON. A hello is at most 4 KiB, a report 1 MiB (`Report::encode` splits a larger one, and values are at most 64 KiB each), and a reply to `2` 1 MiB. The host holds at most 64 commands for a guest that isn't asking, dropping the oldest.

The `guest` feature has the guest's client; `async-host` has the host's half, an async plugin that hands what the guest says to a `Link` and answers its open call from an `Inbox`.

## How it fits

Apps never select it. The shared runtime installs it in `leptoskit dev` builds, and the host merges the guest's route, state, forms, and queries into the session it reports.

## Use it

```toml
# Installed automatically in a `leptoskit dev` build.
```

## Learn more

- [Crate documentation](https://docs.rs/leptoskit_plugin_devtools)
- [leptoskit documentation](https://ifiokjr.github.io/leptoskit/)
- [Repository](https://github.com/ifiokjr/leptoskit)
- [DevTools](https://ifiokjr.github.io/leptoskit/guide/26-devtools.html)
