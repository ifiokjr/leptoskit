# leptoskit_plugin_filesystem

Application-private binary files for native LeptosKit apps and browser OPFS.

```rust,ignore
let files = leptoskit_plugin_filesystem::guest::Filesystem::new()?;
files.mkdir("notes").await?;
files.write("notes/draft", b"hello").await?;
let contents = files.read("notes/draft").await?;
```

Add the crate to the generated native library's dependencies and select `filesystem` in `native.plugins`. The generated registrant uses `rust-factory` and `rust-worker` metadata. iOS supplies its Application Support directory; Android supplies `Context.filesDir`. The plugin owns a `files` subdirectory and opens it as a `cap-std` directory capability. Hosts embedding Rust directly can use `Filesystem::open` with a trusted, existing application-private directory. Unix factory-created storage has mode `0700`.

`read`, `write`, `list`, `mkdir`, `remove`, `stat`, and `rename` use relative UTF-8 paths. Empty paths address the root for `list` and `stat` only. No operation can remove, rename, or replace the root. Absolute paths, parent traversal, ambiguous Windows filenames, NULs, and alternate separators are rejected. The native capability resolver confines every operation even if a symlink is swapped concurrently. Internal directory symlinks may resolve within the capability; symlinks escaping the root cannot be followed. Final symlinks cannot be read or written, and listings omit symlinks and special files. Errors for missing paths and other native IO failures currently map to the bridge's `Native` status; permission failures retain `PermissionDenied`.

Files are limited to 1 MiB per operation, paths to 4096 UTF-8 bytes, and directory listings to 1024 entries and 64 KiB. Oversized results fail explicitly. Lists contain sorted immediate names. Removal is non-recursive. Native writes sync a temporary file and atomically rename it within the same filesystem, so readers observe the old or new complete contents. This does not promise directory-entry durability across power loss. Rename replacement rules follow the platform; use a new destination for portable behavior.

Generated native hosts execute file operations on a bounded Rust worker, without moving the interpreter off its UI thread. Cancelling a guest request discards its response; an OS write already in progress may still finish. Direct Rust calls are synchronous and must be scheduled appropriately by their caller.

Native storage defaults to 64 MiB of file contents and 4096 entries, counting files, directories, and abandoned write temporaries. Trusted hosts can select smaller or larger limits with `Filesystem::with_quota(Quota { max_bytes, max_entries })`. Usage is reconciled from disk under an OS lock before creation or replacement, including across instances and restarts. Overwrites account for the replaced file; deletion and rename remain available when an existing directory exceeds a lowered quota. Quota failures return `TooLarge` before mutation. Atomic replacement needs up to one extra 1 MiB temporary and one extra entry while a write is in progress; leave that headroom in addition to filesystem metadata and allocation overhead. Hosts must keep the directory private and must not remove its reserved lock file.

For example, a native host can keep a small app's files within 8 MiB and 128 entries:

```rust,ignore
use leptoskit_plugin_filesystem::{Filesystem, Quota};

let files = Filesystem::open(app_files_dir)?.with_quota(Quota {
    max_bytes: 8 * 1024 * 1024,
    max_entries: 128,
});
```

The `web` module uses the browser's origin-private filesystem and a `leptoskit` subdirectory, requiring a secure context and OPFS support. Browser writes commit through `createWritable`/`close`; quota failures return `TooLarge`. A failed new file write may leave an empty file entry. Browser rename requires `FileSystemFileHandle.move`; directory moves and missing browser implementations return `Unavailable` without a non-atomic copy/delete fallback. Origin storage can be cleared or evicted by the browser. No browser storage-persistence grant is requested automatically.

This ports a focused application-file surface, informed by [Expo FileSystem](https://docs.expo.dev/versions/latest/sdk/filesystem/) and [Tauri file-system](https://v2.tauri.app/plugin/file-system/). It does not yet provide arbitrary user-selected files, external storage, assets, downloads, watchers, recursive removal, streaming/chunked large files, or full Expo/Tauri API compatibility. Browser access follows the [OPFS API](https://developer.mozilla.org/en-US/docs/Web/API/File_System_API/Origin_private_file_system).
