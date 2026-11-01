<h1 align="center">leptoskit_extension</h1>

<p align="center">
  <strong>leptoskit_extension</strong> is the runtime and typed WebExtensions bindings behind leptoskit browser extensions for Chromium, Firefox, and Safari.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://docs.rs/leptoskit_extension"><strong>Docs</strong></a> ·
  <a href="https://github.com/ifiokjr/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<p align="center">
  <a href="https://crates.io/crates/leptoskit_extension"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_extension.svg"></a>
  <a href="https://docs.rs/leptoskit_extension"><img alt="docs.rs" src="https://docs.rs/leptoskit_extension/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<br>

## Overview

The runtime behind leptoskit's browser extensions, and typed bindings for the WebExtensions APIs that Chromium, Firefox, and Safari share.

An app turns it on with `leptoskit`'s `extension` feature, names its entry points with `leptoskit::extension::entry!`, and builds with `leptoskit build --platform extension`. Every API returns `Error::Unavailable` in a browser that lacks it, rather than throwing, and every event listener is removed when its handle is dropped.

## How it fits

Apps enable the `extension` feature, name entry points with `entry!`, and build with `--platform extension`. APIs return `Unavailable` instead of throwing, and listeners are removed when their handle drops.

## Use it

```toml
[dependencies]
leptoskit = { version = "0.1", features = ["extension"] }
```

## Learn more

- [Crate documentation](https://docs.rs/leptoskit_extension)
- [leptoskit documentation](https://ifiokjr.github.io/leptoskit/)
- [Repository](https://github.com/ifiokjr/leptoskit)
- [Browser extensions](https://ifiokjr.github.io/leptoskit/platforms/extension.html)
