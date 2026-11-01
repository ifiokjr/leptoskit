<h1 align="center">leptoskit_rive_runtime</h1>

<p align="center">
  <strong>leptoskit_rive_runtime</strong> is the pinned, offline C++ dependency used by `leptoskit_rive` native playback.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://docs.rs/leptoskit_rive_runtime"><strong>Docs</strong></a> ·
  <a href="https://github.com/ifiokjr/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<p align="center">
  <a href="https://crates.io/crates/leptoskit_rive_runtime"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_rive_runtime.svg"></a>
  <a href="https://docs.rs/leptoskit_rive_runtime"><img alt="docs.rs" src="https://docs.rs/leptoskit_rive_runtime/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<br>

## Overview

The pinned, offline C++ dependency used by `leptoskit_rive` native playback. Applications use `leptoskit_rive`; this crate exposes no application API. Its separate Cargo build lets changes to the Rust player or image adapter reuse the C++ compilation cache.

Native builds require a C++17 compiler and the target platform's SDK. Browser Wasm and WASI never compile the native sources. Source provenance and its MIT license are included in `vendor/`.

The private C++ library uses optimization level 1 in development and 2 in release, without C++ debug symbols. Rust debug information and upstream runtime assertions retain their normal profile behavior. Supply compiler debug flags explicitly when investigating upstream C++ code.

## How it fits

Applications use `leptoskit_rive`; this crate exposes no application API. Its separate Cargo build lets changes to the Rust player or image adapter reuse the C++ compilation cache.

## Use it

```toml
[dependencies]
leptoskit_rive = "0.1"
```

## Learn more

- [Crate documentation](https://docs.rs/leptoskit_rive_runtime)
- [leptoskit documentation](https://ifiokjr.github.io/leptoskit/)
- [Repository](https://github.com/ifiokjr/leptoskit)
- [Rive](https://ifiokjr.github.io/leptoskit/guide/20-rive.html)
