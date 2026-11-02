<h1 align="center">leptoskit_plugin</h1>

<p align="center">
  <a href="https://ifiokjr.github.io/leptoskit/"><img src="https://raw.githubusercontent.com/ifiokjr/leptoskit/main/docs/src/assets/mark.svg" alt="leptoskit" width="64"></a>
</p>

<p align="center">
  <strong>leptoskit_plugin</strong> is the build-hook system shared by leptoskit plugins and leptos plugins: feature flags, environment variables, asset directories, and post-build reports.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://docs.rs/leptoskit_plugin"><strong>Docs</strong></a> ·
  <a href="https://github.com/ifiokjr/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<p align="center">
  <a href="https://crates.io/crates/leptoskit_plugin"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_plugin.svg"></a>
  <a href="https://docs.rs/leptoskit_plugin"><img alt="docs.rs" src="https://docs.rs/leptoskit_plugin/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<br>

## Overview

Two kinds of plugins ride the same `leptoskit.toml` declaration:

- **leptoskit plugins** implement the `LeptoskitPlugin` trait — `before_build`, `after_build`, `before_serve` hooks that add cargo features, environment variables, asset directories, and post-build reporting. Built-ins ship with the crate (`env_manifest`, `size_report`); local plugin crates declare a `path` and compile into the tooling.
- **leptos plugins** declare `leptos = true` and are forwarded to the app build as cargo features, which is how the leptos plugin ecosystem activates. Their options table survives into the build untouched.

## How it fits

A plugin crate implements `LeptoskitPlugin` and is declared in `leptoskit.toml`; leptos plugins are forwarded to the app build as cargo features. Runtime native plugins use the bridge instead; this is the build-time half.

## Use it

```toml
[[plugins]]
name = "size_report"

[[plugins]]
name = "leptos_meta"
leptos = true
```

## Learn more

- [Crate documentation](https://docs.rs/leptoskit_plugin)
- [leptoskit documentation](https://ifiokjr.github.io/leptoskit/)
- [Repository](https://github.com/ifiokjr/leptoskit)
- [Plugins](https://ifiokjr.github.io/leptoskit/platforms/plugins.html)
