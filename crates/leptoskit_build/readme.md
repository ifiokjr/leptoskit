<h1 align="center">leptoskit_build</h1>

<p align="center">
  <strong>leptoskit_build</strong> is the build-script helper behind `platform_module!`: it records which platform-specific modules exist and cfg-selects them while one app keeps a single module tree.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://docs.rs/leptoskit_build"><strong>Docs</strong></a> ·
  <a href="https://github.com/ifiokjr/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<p align="center">
  <a href="https://crates.io/crates/leptoskit_build"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_build.svg"></a>
  <a href="https://docs.rs/leptoskit_build"><img alt="docs.rs" src="https://docs.rs/leptoskit_build/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<br>

## Overview

Build-script support for one Leptoskit application with platform-specific Rust modules. Add it to the application's build dependencies.

Call it from `build.rs`:

```rust
fn main() -> Result<(), leptoskit_build::Error> {
	leptoskit_build::platforms()
}
```

Then register a shared module in the application:

```rust
leptoskit::platform_module!(pub mod profile = "routes/profile");
```

The path is relative to the application's `src` directory. `routes/profile.rs` is the shared fallback. Adding `routes/profile.ios.rs` automatically replaces it when building with the `ios` feature. The supported features are `web`, `ios`, `android`, `macos`, `windows`, and `linux`; one application build selects at most one. Builds without a platform feature use shared sources.

The helper watches the source directory recursively, so adding or deleting an override takes effect on the next Cargo build. It communicates the selected device through `LEPTOSKIT_PLATFORM`, keeping iOS and Android distinct even though both application guests compile to Wasm. It does not copy application sources or generate a second module tree.

For nested modules, register the child in both the shared parent and its overrides:

```rust
// src/routes/profile.rs and profile.<platform>.rs
leptoskit::platform_module!(pub mod details = "routes/profile/details");
```

The selector uses Rust's `#[path]` rules. A plain `mod details;` in a selected profile file searches beside that file (`src/routes/details.rs`), rather than inside `src/routes/profile/`. An explicit child registration preserves the intended directory while keeping `self`, `super`, and original compiler source locations.

## How it fits

Generated apps call it from `build.rs`. It lets a shared module be replaced by a `.macos.rs`, `.ios.rs`, or `.web.rs` file without changing the imports in the rest of the app.

## Use it

```toml
[build-dependencies]
leptoskit_build = "0.1.0"
```

## Learn more

- [Crate documentation](https://docs.rs/leptoskit_build)
- [leptoskit documentation](https://ifiokjr.github.io/leptoskit/)
- [Repository](https://github.com/ifiokjr/leptoskit)
- [Creating a project](https://ifiokjr.github.io/leptoskit/guide/02-creating.html)
