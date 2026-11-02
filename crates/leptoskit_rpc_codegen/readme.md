<h1 align="center">leptoskit_rpc_codegen</h1>

<p align="center">
  <strong>leptoskit_rpc_codegen</strong> holds the RPC specification behind leptoskit server functions and generates typed clients from it for TypeScript, Python, Go, Rust, Dart, and the command line.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://docs.rs/leptoskit_rpc_codegen"><strong>Docs</strong></a> ·
  <a href="https://github.com/ifiokjr/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<p align="center">
  <a href="https://crates.io/crates/leptoskit_rpc_codegen"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_rpc_codegen.svg"></a>
  <a href="https://docs.rs/leptoskit_rpc_codegen"><img alt="docs.rs" src="https://docs.rs/leptoskit_rpc_codegen/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<br>

## Overview

The specification is plain `serde` data: one `ApiFunction` per `#[leptoskit_rpc::api]` function — route, HTTP encodings, docs, and a JSON Schema for every argument and the return value — plus the shared schemas. The server side (`leptoskit_rpc`) writes it; this crate reads it back and generates clients that speak the RPC's own protocol: TypeScript over `fetch`, Python over the standard library, Go over `net/http`, Rust over `reqwest`, Dart over `package:http`, a clap command line with one command per function, and the OpenAPI 3.1 document.

The generators sit behind the default `generators` feature; the server binary depends on this crate without it, for the specification types alone. The `leptoskit` CLI drives everything through `leptoskit rpc`.

Generated names never shadow the client's own: a schema named `Client`, `Error`, or `Record`, or a function named `call` or `headers`, gets the first free numeric suffix (`Client2`, `call2`) in the targets where its name is taken, and references follow the renamed type.

## How it fits

The server writes the specification; the CLI reads it back and generates clients that speak the RPC's own protocol, plus an OpenAPI 3.1 document. The generators sit behind a default feature, so the server binary can depend on the types alone.

## Use it

```bash
leptoskit rpc generate    # write clients for the configured targets
```

## Learn more

- [Crate documentation](https://docs.rs/leptoskit_rpc_codegen)
- [leptoskit documentation](https://ifiokjr.github.io/leptoskit/)
- [Repository](https://github.com/ifiokjr/leptoskit)
- [Sharing the RPC](https://ifiokjr.github.io/leptoskit/guide/13-sharing-the-rpc.html)
