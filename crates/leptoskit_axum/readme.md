<h1 align="center">leptoskit_axum</h1>

<p align="center">
  <a href="https://ifiokjr.github.io/leptoskit/"><img src="https://raw.githubusercontent.com/ifiokjr/leptoskit/main/docs/src/assets/mark.svg" alt="leptoskit" width="64"></a>
</p>

<p align="center">
  <strong>leptoskit_axum</strong> is the Axum server toolkit every leptoskit deployment shares: static files with precompressed copies, SPA fallback, health checks, and code push routes.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://docs.rs/leptoskit_axum"><strong>Docs</strong></a> ·
  <a href="https://github.com/ifiokjr/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<p align="center">
  <a href="https://crates.io/crates/leptoskit_axum"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_axum.svg"></a>
  <a href="https://docs.rs/leptoskit_axum"><img alt="docs.rs" src="https://docs.rs/leptoskit_axum/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<br>

## Overview

Axum is the one supported server. This crate wraps the pieces every deployment needs: static asset serving with pre-compressed brotli and gzip and correct wasm MIME types, an SPA fallback to `index.html` for `GET`/`HEAD` requests whose last path segment has no file extension (missing assets stay `404`), health checks at `/_leptoskit/health`, and the code push registry under `/api/codepush`. SSR and leptos server functions mount through `leptos_axum` in the app itself; this crate stays leptos-free to keep the server binary small. A server-rendered app mounts `static_files` behind its routes to serve the built site the same way, merges `Readiness::routes` for `/healthz` and `/readyz`, and serves with `serve_listener_with` and a `GracefulShutdown`, which fails readiness on `SIGTERM` before it stops accepting and finishes the requests in flight.

## How it fits

The generated server mounts it beside `leptos_axum`; apps that serve their own assets use the same functions. It stays leptos-free so the server binary only pays for what it uses.

## Use it

```rust,ignore
use leptoskit_axum::{serve_listener_with, static_files};
```

## Learn more

- [Crate documentation](https://docs.rs/leptoskit_axum)
- [leptoskit documentation](https://ifiokjr.github.io/leptoskit/)
- [Repository](https://github.com/ifiokjr/leptoskit)
- [Deploying](https://ifiokjr.github.io/leptoskit/guide/24-deploying.html)
