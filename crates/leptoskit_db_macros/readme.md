<h1 align="center">leptoskit_db_macros</h1>

<p align="center">
  <a href="https://ifiokjr.github.io/leptoskit/"><img src="https://raw.githubusercontent.com/ifiokjr/leptoskit/main/docs/src/assets/mark.svg" alt="leptoskit" width="64"></a>
</p>

<p align="center">
  <strong>leptoskit_db_macros</strong> provides `#[db::table]`, `#[derive(db::Enum)]`, and `#[derive(db::Scalar)]` behind leptoskit's database layer.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://docs.rs/leptoskit_db_macros"><strong>Docs</strong></a> ·
  <a href="https://github.com/ifiokjr/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<p align="center">
  <a href="https://crates.io/crates/leptoskit_db_macros"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_db_macros.svg"></a>
  <a href="https://docs.rs/leptoskit_db_macros"><img alt="docs.rs" src="https://docs.rs/leptoskit_db_macros/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<br>

## Overview

Apps use these through `leptoskit::db` (or `leptoskit_db`); see that crate for what they generate. Every mistake the macros can catch is a compile error that says what to write instead: a table without an explicit name, a derive that would leak secrets, a missing key, an invalid regular expression.

## How it fits

Apps use them through `leptoskit::db` or `leptoskit_db`; this crate is the macro implementation. Every mistake it can catch is a compile error that says what to write instead.

## Use it

```rust,ignore
#[leptoskit_db::table]
pub struct Note {
    #[key]
    pub id: NoteId,
    pub body: String,
}
```

## Learn more

- [Crate documentation](https://docs.rs/leptoskit_db_macros)
- [leptoskit documentation](https://ifiokjr.github.io/leptoskit/)
- [Repository](https://github.com/ifiokjr/leptoskit)
- [Database](https://ifiokjr.github.io/leptoskit/guide/22-database.html)
