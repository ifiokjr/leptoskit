<h1 align="center">leptoskit_plugin_sqlite</h1>

<p align="center">
  <strong>leptoskit_plugin_sqlite</strong> is the leptoskit plugin for bounded, application-private SQLite databases with typed bindings and the same bundled engine on every native platform.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://docs.rs/leptoskit_plugin_sqlite"><strong>Docs</strong></a> ·
  <a href="https://github.com/ifiokjr/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<p align="center">
  <a href="https://crates.io/crates/leptoskit_plugin_sqlite"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_plugin_sqlite.svg"></a>
  <a href="https://docs.rs/leptoskit_plugin_sqlite"><img alt="docs.rs" src="https://docs.rs/leptoskit_plugin_sqlite/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<br>

## Overview

An application-private SQLite database implemented in Rust, with the same bundled SQLite engine on iOS, Android, macOS, Windows and Linux. No JavaScript SQL engine or platform channel is needed for database work.

The native host automatically registers `sqlite` when this crate is a direct app dependency and the app grants that plugin. Its `rust-worker` metadata sends every operation to the Rust worker. Registration itself only retains the app data path; opening the database happens on the first worker request.

Interpreter guests enable the `guest` feature and use `guest::Sqlite::{new, execute, query, transaction, close}`. Integers remain signed 64-bit values, blobs remain bytes, and text can contain embedded NULs. Duplicate column names are preserved positionally. Bind positional parameters rather than interpolating user input into SQL.

Each `execute` and explicit transaction batch is atomic. A batch has no outstanding transaction between bridge calls: an unrelated call cannot accidentally join it. Constraint errors, malformed SQL, binding errors and interrupted statements roll back the entire batch. Queries must be read-only; write statements with RETURNING are rejected by the query method. `close` releases the connection and subsequent operations reopen the same file.

The database lives at `<host app data>/sqlite/app.sqlite3`, separate from the filesystem plugin's `files` directory. The host chooses an existing app-owned root; guests cannot supply filenames, URLs, directory paths or SQLite URI flags. The plugin creates its child directory through a directory capability, requires private permissions on Unix (Windows relies on the per-user app data directory's ACLs), and rejects child/database/sidecar symlinks, Windows junctions and hard-linked database files on every platform. It opens SQLite with NOFOLLOW. The trusted OS root is canonicalized first to accommodate system aliases such as macOS `/var` → `/private/var`. Privileged native code able to replace the app's directory or files concurrently is outside this boundary.

SQLite's authorizer rejects ATTACH, DETACH, all guest PRAGMAs, virtual-table creation, file helper functions and extension loading. Transaction SQL belongs to the batch API. Foreign keys and defensive mode are enabled; trusted-schema mode is disabled. Temporary storage stays in memory and the rollback journal stays beside the database. This is ordinary SQLite storage, not encrypted storage.

Requests and results are limited to 1 MiB, each statement to 64 KiB SQL and 256 parameters, results to 1,024 rows and 128 columns, and transactions to 128 statements. Results exceeding a limit fail rather than silently truncating. Use SQL pagination for larger datasets. SQLite has a bounded VM instruction budget per call and a 250 ms lock timeout. Native databases default to a 64 MiB file quota, rounded down to whole database pages; trusted hosts can select it with `Database::with_quota(data_dir, max_bytes)`. SQLite reserves pages before writing, returns `TooLarge` through the bridge when full, and rolls back the entire failed batch. Deleted pages can be reused. The quota is reapplied on every open, and an existing database above the selected budget refuses to open. Leave approximately another database's worth of disk space for the rollback journal and filesystem overhead. Worker cancellation drops an unwanted response but cannot promise a write was cancelled; an already running transaction can commit before cancellation is observed.

Native Rust errors preserve SQLite diagnostics in `DatabaseError`. The common bridge maps permission, busy and size errors to its typed statuses; other SQL errors use its `Native` status.

Browser builds compile the data types and codecs, but no browser SQLite backend is registered. A browser implementation needs an actual SQLite WASM worker and OPFS persistence. FTS/virtual tables, encryption, database import/export, prepared statement handles and live change subscriptions are also not implemented here.

The API covers parameterized writes, reads and transactions from [Expo SQLite](https://docs.expo.dev/versions/latest/sdk/sqlite/) and [Tauri SQL](https://v2.tauri.app/plugin/sql/), with constraints enforced using [SQLite authorization](https://www.sqlite.org/c3ref/set_authorizer.html) and [connection limits](https://www.sqlite.org/limits.html).

## Binary protocol

All integers are little-endian; strings and byte sequences have a u32 length. A statement is SQL bytes, parameter count, then typed values. Value tags are 0=NULL, 1=i64, 2=finite f64, 3=UTF-8 text, 4=blob. Numeric values have a byte-length prefix of 8. Execute results are changes u64 followed by last-insert-rowid i64. Query results are column count, length-prefixed names, row count, and row values. Transaction requests contain a count and that many statements; responses contain a count and length-prefixed 16-byte execute results. CLOSE accepts and returns an empty payload. Decoders reject trailing or truncated bytes and unknown tags.

## How it fits

Add `leptoskit_plugin_sqlite` as a direct dependency and select the `sqlite` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::Sqlite` rather than resolving the plugin on every call. The database lives in Rust, not a platform channel or JavaScript engine, so iOS, Android, macOS, Windows, and Linux behave the same; the browser has no SQLite plugin.

## Use it

```rust,no_run
use leptoskit_plugin_sqlite::{Database, Statement, Value};

# fn example(app_data: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
let mut database = Database::new(app_data)?;
database.execute(&Statement::new(
    "CREATE TABLE IF NOT EXISTS notes (id INTEGER PRIMARY KEY, body TEXT NOT NULL)",
    vec![],
))?;
database.transaction(&[
    Statement::new("INSERT INTO notes(body) VALUES (?)", vec![Value::Text("Hello".into())]),
    Statement::new("INSERT INTO notes(body) VALUES (?)", vec![Value::Text("World".into())]),
])?;
let notes = database.query(&Statement::new(
    "SELECT id, body FROM notes ORDER BY id LIMIT ?",
    vec![Value::Integer(20)],
))?;
assert_eq!(notes.rows.len(), 2);
# Ok(())
# }
```

## Learn more

- [Crate documentation](https://docs.rs/leptoskit_plugin_sqlite)
- [leptoskit documentation](https://ifiokjr.github.io/leptoskit/)
- [Repository](https://github.com/ifiokjr/leptoskit)
- [Plugin inventory](https://ifiokjr.github.io/leptoskit/platforms/plugin-inventory.html)
