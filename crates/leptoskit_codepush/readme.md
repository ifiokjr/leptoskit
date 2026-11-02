<h1 align="center">leptoskit_codepush</h1>

<p align="center">
  <a href="https://ifiokjr.github.io/leptoskit/"><img src="https://raw.githubusercontent.com/ifiokjr/leptoskit/main/docs/src/assets/mark.svg" alt="leptoskit" width="64"></a>
</p>

<p align="center">
  <strong>leptoskit_codepush</strong> provides signed, content-hashed over-the-air releases for Wasm bundles, with a single-file container, binary deltas, a static channel layout, and a client that verifies every byte.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://docs.rs/leptoskit_codepush"><strong>Docs</strong></a> ·
  <a href="https://github.com/ifiokjr/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<p align="center">
  <a href="https://crates.io/crates/leptoskit_codepush"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_codepush.svg"></a>
  <a href="https://docs.rs/leptoskit_codepush"><img alt="docs.rs" src="https://docs.rs/leptoskit_codepush/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<br>

## Overview

A release is a `BundleManifest` — content-hashed, gzip-compressed files packed into a single `.lkb` container. Releases live in a static layout: immutable files plus one `index.json` per channel naming the newest release, so any file server, object store, or CDN can host a channel with no server running. Clients read the index and fetch either the full bundle or a binary delta addressed by the hash of exactly what they already run. Every release is signed with the publisher's ed25519 key, and a client stages only releases that are signed by a key it trusts, meant for its app and channel, and newer than what it runs. Every download is hash-verified against that signed manifest before it stages.

- `create_delta` / `apply_delta` — block-indexed binary deltas between containers. Containers compress each file in chunks of about 64 KiB that come out the same whenever their input does, so an update that changes part of a module ships as the chunks it touched;
- `ReleaseStore` — the on-disk registry in the static layout, computing a delta against the previous release on ingest and keeping it when it is at most three quarters of the full container;
- `publish_channel` / `pull_channel` / `Bucket` — upload a channel to an object store, `index.json` last, and seed a store from one after verifying the newest release;
- `codepush_router` (feature `server`) — Axum routes serving the same static files, plus the older check/download/delta endpoints;
- `ReleaseSigningKey` / `TrustedKeys` — publisher signing and the key set an app shell pins, with room to rotate;
- `CodePushClient` — the updater, transport-agnostic so each platform shell brings its own HTTP stack.

## How it fits

`leptoskit codepush` uses it to pack and publish, `leptoskit_axum` serves it, and `leptoskit_wasm` consumes it on the device. It is the React Native update model applied to the interface bundle.

## Use it

```bash
leptoskit codepush release   # pack dist/ into a versioned release
leptoskit codepush publish   # upload releases to an S3-compatible bucket
```

## Learn more

- [Crate documentation](https://docs.rs/leptoskit_codepush)
- [leptoskit documentation](https://ifiokjr.github.io/leptoskit/)
- [Repository](https://github.com/ifiokjr/leptoskit)
- [Code push](https://ifiokjr.github.io/leptoskit/guide/05-code-push.html)
