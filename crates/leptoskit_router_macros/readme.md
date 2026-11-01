<h1 align="center">leptoskit_router_macros</h1>

<p align="center">
  <a href="https://ifiokjr.github.io/leptoskit/"><img src="https://raw.githubusercontent.com/ifiokjr/leptoskit/main/docs/src/assets/mark.svg" alt="leptoskit" width="64"></a>
</p>

<p align="center">
  <strong>leptoskit_router_macros</strong> provides the `#[derive(Route)]` macro that reads a route struct's fields from the URL by name.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://docs.rs/leptoskit_router_macros"><strong>Docs</strong></a> ·
  <a href="https://github.com/ifiokjr/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<p align="center">
  <a href="https://crates.io/crates/leptoskit_router_macros"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_router_macros.svg"></a>
  <a href="https://docs.rs/leptoskit_router_macros"><img alt="docs.rs" src="https://docs.rs/leptoskit_router_macros/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<br>

## Overview

An implementation detail of [`leptoskit_router`](../leptoskit_router), which re-exports the derive. It implements `RouteParams` for a route struct: each field is read from the URL by name, and its type decides how — `Option<T>` may be absent, `Vec<T>` takes every value, and any other `T` is required unless the field is `#[route(default)]`.

## How it fits

It is an implementation detail of `leptoskit_router`, which re-exports the derive. `Option<T>` may be absent, `Vec<T>` takes every value, and other types are required unless the field is `#[route(default)]`.

## Use it

```rust,ignore
#[derive(leptoskit_router::Route)]
struct UserRoute {
    id: u64,
}
```

## Learn more

- [Crate documentation](https://docs.rs/leptoskit_router_macros)
- [leptoskit documentation](https://ifiokjr.github.io/leptoskit/)
- [Repository](https://github.com/ifiokjr/leptoskit)
- [File routes](https://ifiokjr.github.io/leptoskit/guide/10-file-routes.html)
