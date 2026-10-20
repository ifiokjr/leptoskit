# Document picker

Imports one document through iOS `UIDocumentPickerViewController`, macOS `NSOpenPanel`, Android Storage Access Framework, or an HTML file input. The native adapters register automatically when selected in the app configuration. Apple registration supplies the owning `NativePluginContext`; Android uses the generated activity's `NativeActivityResultHost`. Custom hosts must supply these same capabilities.

```rust,ignore
let document = leptoskit_plugin_document_picker::pick(
    leptoskit_plugin_document_picker::Options::new(1024 * 1024)?,
).await?;
```

Enable `guest` for native guest calls. Browser applications call `web::pick(options)` directly from a trusted user action. Its future must be polled during that action, before user activation expires. Browsers must support `showPicker` and the input `cancel` event. Dropping the browser future removes listeners and its input but cannot forcibly dismiss an operating system dialog. `web::read_file` also accepts an existing selected `File`.

The fixed maximum and default content limit is **4 MiB**, with each UTF-8 metadata field capped at 4096 bytes. Native replies contain three little-endian u32-length-prefixed fields: display name, MIME type (empty when unknown), and raw bytes. Rust decodes content as a shared slice of the original reply allocation. No JSON or base64 is used. Filename and MIME values are untrusted provider metadata, never content validation or a safe destination path.

Reads run off the native UI thread, check cancellation between bounded chunks, reject oversized files even when a provider lies about its size, and close resources on every exit. Apple coordinates external reads and balances security-scoped access; Android opens a transient content URI without requesting storage permissions or retaining a persistable grant. A stalled cloud provider may delay cancellation while an OS read is blocked; the host retains that job until cleanup finishes. Cancellation, permission failures, unavailable presenters, and native failures are errors, not empty successful documents.

This first port selects a single ordinary readable file. It does not implement directories, exporting, multi-selection, MIME filters, virtual Android documents, or persistent bookmarks. Sandboxed macOS apps must enable user-selected read access in their app-owned entitlements; iCloud configuration also remains app-owned. Native picker UI is not opened by the automated tests; tests use synthetic owned files and streams.

Sources: [Expo DocumentPicker](https://docs.expo.dev/versions/latest/sdk/document-picker/), [Tauri dialog](https://v2.tauri.app/plugin/dialog/), [Apple document access](https://developer.apple.com/library/archive/documentation/FileManagement/Conceptual/DocumentPickerProgrammingGuide/AccessingDocuments/AccessingDocuments.html), [Android shared documents](https://developer.android.com/training/data-storage/shared/documents-files), [browser input cancellation](https://developer.mozilla.org/en-US/docs/Web/API/HTMLInputElement/cancel_event).

Windows and Linux desktop hosts can call `desktop::pick(&parent_window, options)`; macOS also supports this direct API. It reuses the existing `leptoskit_plugin_dialog` backend and reads selected content on a worker thread. At most four reads can run per process; cancelled blocked reads retain their admission slot until they finish. Keep the explicit parent alive until the OS picker closes, including if a future is dropped. The underlying native backend cannot distinguish cancellation from every presentation failure; either returns an error. This direct adapter is not an automatically registered synchronous guest handler. Automated native desktop tests read synthetic owned fixtures; Windows/Linux picker windows have not been exercised here.
