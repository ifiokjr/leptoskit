<h1 align="center">leptoskit_service_worker</h1>

<p align="center">
  <strong>leptoskit_service_worker</strong> registers and talks to browser service workers from leptoskit web apps: caching strategies, page↔worker messages, and push subscriptions.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://docs.rs/leptoskit_service_worker"><strong>Docs</strong></a> ·
  <a href="https://github.com/leptoskit/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<!-- {=crateBadges|replace:"CRATE_NAME":"leptoskit_service_worker"} -->

<p align="center">
  <a href="https://crates.io/crates/leptoskit_service_worker"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_service_worker.svg"></a>
  <a href="https://docs.rs/leptoskit_service_worker"><img alt="docs.rs" src="https://docs.rs/leptoskit_service_worker/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<!-- {/crateBadges} -->

<br>

## Overview

Typed Rust bindings for the page side of a Progressive Web App's service worker. Register a script, wait until it controls the page, post messages, open Cache Storage, describe caching strategies for the worker to follow, and manage Web Push subscriptions.

Calls return `Error::Unavailable` outside a secure browser context, so the same app code compiles and its tests run on any host. This crate does not install a service worker script for you: ship one under your web assets (or generate one) and pass its URL to `register`.

## Use it

```toml
[dependencies]
leptoskit = { version = "0.3", features = ["service-worker"] }
```

## Learn more

- [Crate documentation](https://docs.rs/leptoskit_service_worker)
- [Service workers](https://leptoskit.dev/docs/platforms/web.html#service-workers)
- [Repository](https://github.com/leptoskit/leptoskit)
