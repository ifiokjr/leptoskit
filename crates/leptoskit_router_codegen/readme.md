<h1 align="center">leptoskit_router_codegen</h1>

<p align="center">
  <a href="https://ifiokjr.github.io/leptoskit/"><img src="https://raw.githubusercontent.com/ifiokjr/leptoskit/main/docs/src/assets/mark.svg" alt="leptoskit" width="64"></a>
</p>

<p align="center">
  <strong>leptoskit_router_codegen</strong> reads a leptoskit `src/routes/` directory and writes its typed `routes.rs`.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://docs.rs/leptoskit_router_codegen"><strong>Docs</strong></a> ·
  <a href="https://github.com/ifiokjr/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<p align="center">
  <a href="https://crates.io/crates/leptoskit_router_codegen"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_router_codegen.svg"></a>
  <a href="https://docs.rs/leptoskit_router_codegen"><img alt="docs.rs" src="https://docs.rs/leptoskit_router_codegen/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<br>

## Overview

The `leptoskit routes` command and the CLI's build pipeline use this crate; apps never depend on it.

- `generate(dir)` reads the routes directory and returns the generated module with its route table, or every problem with its file and line: names that aren't routes, route files without a page, route structs that don't match their path, and routes that compete for the same URLs. Every file name becomes a valid module and `Route` variant, escaped the same way for both (`404.rs` is `_404`), and a route with optional segments is ordered once for each combination of them present and absent, so precedence compares what a URL actually matches.
- `write(output, &generated)` writes the module only when it changed, and never overwrites a file leptoskit didn't write.
- `status(output, &generated)` compares without writing, for `leptoskit routes --check`. Formatting and item order don't count as changes, so rustfmt can format the committed file.

The generator reads file names and the names of top-level items with `syn`. It never runs route code.

## How it fits

The `leptoskit routes` command and the CLI's build pipeline use it; apps never depend on it. It reports every problem with its file and line, and never overwrites a file leptoskit did not write.

## Use it

```bash
leptoskit routes           # write src/routes.rs
leptoskit routes --check   # fail if it is stale
```

## Learn more

- [Crate documentation](https://docs.rs/leptoskit_router_codegen)
- [leptoskit documentation](https://ifiokjr.github.io/leptoskit/)
- [Repository](https://github.com/ifiokjr/leptoskit)
- [File routes](https://ifiokjr.github.io/leptoskit/guide/10-file-routes.html)
