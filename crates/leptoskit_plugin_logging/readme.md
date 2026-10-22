# leptoskit_plugin_logging

Bounded structured application logging inspired by [Tauri's severity, sink, and rotation controls](https://v2.tauri.app/plugin/logging/) and [Electron's application log directory](https://www.electronjs.org/docs/latest/api/app#appsetapplogspathpath). There is no global logger installation or remote logging backend dependency.

Select `"logging"` in `native.plugins` and add this crate to the host. Cargo metadata selects the app-directory factory and Rust worker. Registration does no I/O; the first accepted record opens `<app data>/logs` on the worker.

This crate requires Rust 1.89 for the standard library's file locking API. Android uses Bionic's nonblocking `flock` because Rust 1.90 does not implement `File::try_lock` on Android. Both paths keep an exclusive advisory lock until the owning file descriptor closes; unsupported locking is an error, never permission to write without a lock.

```rust,ignore
use leptoskit_plugin_logging::{Level, Record, guest::Logger};
let logger = Logger::new()?;
logger.write(&Record {
    level: Level::Info,
    target: "sync".into(),
    message: "Synchronization completed".into(),
    fields: [("items".into(), "12".into())].into(),
}).await?;
logger.flush().await?;
```

Records have severity, a component target, a message, and ordered string fields. Native output adds a host timestamp in Unix milliseconds and writes escaped JSON lines. Targets and fields are data, never paths or format strings. Native hosts configure sinks and minimum severity; guests cannot change destinations.

The default retains four files of at most 1 MiB each, including `current.jsonl`, with an `Info` threshold. Archives are `1.jsonl` (newest) through `3.jsonl`. Hosts may choose 1–16 files and 128 bytes–16 MiB per file, and optionally mirror to process stderr. A nonblocking advisory writer lock rejects another logger using the same root with `Busy`; this can be retried after the owner closes. The lock file is additional to retained logs. Lowering retention removes older managed archives on the next accepted record. Existing oversized files are rejected instead of being silently deleted.

Files are accessed through an open directory capability. Symlinks and special files are rejected, and Unix directories/files use private creation modes. Rotation syncs the active file before renaming it. `flush` syncs active file data and flushes stderr; directory metadata durability on power loss is not promised. No application-side background queue or timer accumulates records.

An input error leaves the logger usable. A sink I/O failure disables it, because one sink or part of a line may already have been written. Rotation is not a transaction across all retained files: a failure midway may leave gaps in the archive sequence. Recreate the logger after addressing the underlying error. An incomplete final line from an earlier failure is reported rather than joined to a new record. Explicit `close` attempts flush and releases resources even when it fails; repeated closes succeed. Drop releases handles without an implicit, unreportable sync. Logging to multiple sinks is not exactly-once delivery.

The browser adapter emits structured objects through console debug/info/warn/error. It does not replace the global console, persist records, or offer a browser flush acknowledgement. Native stderr is not an OSLog or Android Logcat integration. Native file output is available on desktop, iOS, and Android through Rust.

Each record permits a 128-byte target, a 16 KiB message, and 32 fields with 64-byte names and 1 KiB values. Complete binary input is capped at 64 KiB. Control characters in labels and duplicate field names are rejected. Messages may contain newlines; file output escapes them, preserving one event per line. No automatic redaction is performed; applications must avoid logging credentials.

Wire methods: `0` writes a binary record and returns a `u32` accepted flag (`0` filtered, `1` written); `1` flushes; `2` closes. Record input contains the severity `u32` (`trace=0` through `error=4`), length-prefixed UTF-8 target/message, field count, and length-prefixed key/value pairs. All decoding is strict.
