<h1 align="center">leptoskit_devtools</h1>

<p align="center">
  <strong>leptoskit_devtools</strong> contains the agents that report a running app to the local DevTools hub: the web page, the native host, and the Wasm guest.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://docs.rs/leptoskit_devtools"><strong>Docs</strong></a> ·
  <a href="https://github.com/ifiokjr/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<p align="center">
  <a href="https://crates.io/crates/leptoskit_devtools"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_devtools.svg"></a>
  <a href="https://docs.rs/leptoskit_devtools"><img alt="docs.rs" src="https://docs.rs/leptoskit_devtools/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<br>

## Overview

The DevTools agents. The `host` agent reports a native host to the local hub: a `Reporter` opens a session, mirrors each guest's tree from the mutations its renderer applies, sends snapshots, patches, and commits from a thread of its own, and long-polls for the commands a front end sends back. The `web` agent reports a browser page. The `guest` agent runs in a native Wasm guest and sends the hooks' values through the host's `DevTools` plugin (`leptoskit_plugin_devtools`); `Reporter::with_guests` and `Source::guest` merge them into the host's session and pass the hub's commands for them back.

The hooks hand over what only the app knows: `track_router` for the route, `expose` for named state, `track_form` for a `leptoskit_form` form (the `form` feature), and `track_queries` for a `leptoskit_query` cache (the `query` feature). Outside a `leptoskit dev` build they do nothing.

## How it fits

Under `leptoskit dev`, the CLI starts the hub and these agents open sessions, mirror trees, and carry commands back. Hooks expose the route, named state, forms, and query caches that only the app knows.

## Use it

```rust,ignore
leptoskit_devtools::install();
```

## Learn more

- [Crate documentation](https://docs.rs/leptoskit_devtools)
- [leptoskit documentation](https://ifiokjr.github.io/leptoskit/)
- [Repository](https://github.com/ifiokjr/leptoskit)
- [DevTools](https://ifiokjr.github.io/leptoskit/guide/26-devtools.html)
