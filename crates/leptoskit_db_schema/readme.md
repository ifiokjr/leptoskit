<h1 align="center">leptoskit_db_schema</h1>

<p align="center">
  <a href="https://ifiokjr.github.io/leptoskit/"><img src="https://raw.githubusercontent.com/ifiokjr/leptoskit/main/docs/src/assets/mark.svg" alt="leptoskit" width="64"></a>
</p>

<p align="center">
  <strong>leptoskit_db_schema</strong> holds the schema model, snapshot format, diff, and SQL renderers behind leptoskit's generated database migrations.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://docs.rs/leptoskit_db_schema"><strong>Docs</strong></a> ·
  <a href="https://github.com/ifiokjr/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<p align="center">
  <a href="https://crates.io/crates/leptoskit_db_schema"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_db_schema.svg"></a>
  <a href="https://docs.rs/leptoskit_db_schema"><img alt="docs.rs" src="https://docs.rs/leptoskit_db_schema/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<br>

## Overview

`leptoskit db` and the `leptoskit_db` runtime use this crate; apps never depend on it. Everything in it is pure: no database connection and no files.

- `Schema` describes what a database should contain: tables with typed columns, primary keys, indexes (unique ones included, optionally partial), foreign keys, checks, comments, and namespaces. It also holds Postgres sequences and functions (with a checksum of each body, so a changed body is visible in review), and triggers, including deferred constraint triggers. It reads and writes the `schema.json` snapshot committed beside each migration, and `validate` reports every inconsistency with its table and column.
- `diff(previous, target, renames)` describes, step by step, how to turn one schema into the other. Renames come from `renamed_from` hints, or from a `RenameResolver` when something appeared while something similar disappeared: `RequireHints` fails so that CI never guesses, and `NoRenames` treats the change as new. Indexes and constraints that only changed name, because their table was renamed, are renamed rather than rebuilt. A changed function is replaced in place; a changed trigger is dropped and created again; a SQLite table rebuild recreates the triggers it drops. Every step reports its `Risk` against existing rows, so that data-destroying migrations need explicit consent.
- `render(plan, previous, target)` writes the plan as SQL for its dialect. Postgres alters in place, adds foreign keys once every table exists, drops tables in one statement, and creates routines and triggers in dependency order. SQLite uses `STRICT` tables with checks that hold each column to its type, rebuilds a table whenever `ALTER TABLE` can't make the change, using SQLite's documented procedure with foreign keys off and checked before commit, and creates triggers with their statements inline.
- `live` holds the `live` pack's objects: the change-capture functions for Postgres, the sequences that order changes and versions, and the capture triggers each live table gets. `leptoskit_db`'s schema builder adds them when an app has a `#[db::table(live)]` table.

The baseline for a new migration is always the previous migration's snapshot, never a live database, so generating one gives the same SQL on every machine. The test suite applies every scenario to real SQLite and Postgres databases and checks that a migrated schema matches one created from scratch, with every row kept.

## How it fits

`leptoskit db` and the `leptoskit_db` runtime use it; apps never depend on it. It is pure - no connection and no files - and renders Postgres and SQLite SQL from the same plan, with each step's risk assessed against existing rows.

## Use it

```bash
leptoskit db generate     # diff schema.json and write a migration
```

## Learn more

- [Crate documentation](https://docs.rs/leptoskit_db_schema)
- [leptoskit documentation](https://ifiokjr.github.io/leptoskit/)
- [Repository](https://github.com/ifiokjr/leptoskit)
- [Database](https://ifiokjr.github.io/leptoskit/guide/22-database.html)
