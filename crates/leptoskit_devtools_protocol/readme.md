<h1 align="center">leptoskit_devtools_protocol</h1>

<p align="center">
  <strong>leptoskit_devtools_protocol</strong> holds the versioned serde messages shared by the DevTools agents, hub, interface, and MCP tools.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://docs.rs/leptoskit_devtools_protocol"><strong>Docs</strong></a> ·
  <a href="https://github.com/ifiokjr/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<p align="center">
  <a href="https://crates.io/crates/leptoskit_devtools_protocol"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_devtools_protocol.svg"></a>
  <a href="https://docs.rs/leptoskit_devtools_protocol"><img alt="docs.rs" src="https://docs.rs/leptoskit_devtools_protocol/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<br>

## Overview

The serde types shared by leptoskit's DevTools agents, hub, interface, and MCP tools. It has no platform code: agents on the web, native hosts, and guests all speak the same messages.

Every message carries [`PROTOCOL_VERSION`]. A hub refuses a session that speaks another version and names both versions in the error.

## How it fits

It has no platform code: the browser, native host, guest, and front end all speak the same protocol. A version mismatch is refused and both versions are named.

## Use it

```rust
use leptoskit_devtools_protocol::PROTOCOL_VERSION;
```

## Learn more

- [Crate documentation](https://docs.rs/leptoskit_devtools_protocol)
- [leptoskit documentation](https://ifiokjr.github.io/leptoskit/)
- [Repository](https://github.com/ifiokjr/leptoskit)
- [DevTools](https://ifiokjr.github.io/leptoskit/guide/26-devtools.html)
