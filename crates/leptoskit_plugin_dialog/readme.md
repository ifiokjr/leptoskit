# leptoskit_plugin_dialog

Native asynchronous file, directory, save, and message dialogs for macOS, Windows, and Linux. The browser backend selects files through the File System Access API and reads them with an explicit byte limit.

```rust,ignore
use leptoskit_plugin_dialog::desktop::{FileDialog, FileOptions};
use leptoskit_plugin_dialog::FileFilter;

let options = FileOptions {
    title: "Open an image".into(),
    filters: vec![FileFilter::new("Images", ["png", "jpg"])?],
    ..FileOptions::default()
};
let selected = FileDialog::new(&window, options)?.open_file().await;
```

The dialog borrows its parent window until completion. Start it on the UI thread of a running application, and keep that event loop running while awaiting the result. On macOS, rfd can fall back to blocking when no `NSApplication` exists. There is deliberately no synchronous `leptoskit_bridge::Plugin` implementation: host applications must await these operations outside the guest interpreter.

Paths remain native `PathBuf` values, including non-Unicode paths. A save dialog selects a destination without writing or reserving it. Extension filters guide the OS picker; callers still validate the selected content. The native rfd backend returns `None` both on dismissal and on some native failures, so this crate reports no selection rather than claiming it always means cancellation. The desktop message API exposes rfd's typed button and result enums.

Linux application packages need a supported XDG desktop portal and Zenity. Browser file selection requires HTTPS, user activation, and `showOpenFilePicker` support; unsupported browsers return `Unavailable`. Browser results are `web_sys::File` capabilities with no local filesystem path. Browser save, directory selection, and message dialogs are not implemented.

## Native host integration still required

This crate is usable directly by Rust desktop hosts and browser apps. It does not advertise automatic native registration or a guest API yet. The generated native hosts currently register synchronous Rust handlers; dialogs require an asynchronous request adapter that retains the live parent window, cancels or discards completions after host shutdown, and completes calls on the bridge's owning thread.

iOS document selection additionally needs a presenting `UIViewController`, `UIDocumentPickerDelegate` callbacks, cancellation on host close, and explicit handling of security-scoped URLs. Android document selection needs lifecycle registration of Activity Result launchers for `OpenDocument`, `OpenMultipleDocuments`, `OpenDocumentTree`, and `CreateDocument`; results are content URIs, with persisted grants handled explicitly. A bare `Context` is not a replacement for these lifecycle hooks. Mobile handlers are not claimed.

## Real-OS verification

`examples/native_fixture.rs` is a Win32 or AppKit window whose buttons present this plugin's file chooser and message box. The `native UI` workflow's Windows and macOS jobs drive it with `leptoskit_testing`'s `dialog_plugin_journey` example, through NovaWindows and Mac2 respectively. The journey answers both dialogs with the operating system's own Open, Cancel and OK buttons, asserts the result returned to the app, and keeps screenshots of every dialog. A worker thread awaits each dialog while the window keeps pumping messages: Windows enables and disables a dialog's owner synchronously, so awaiting on the owner's thread would deadlock. On macOS, rfd begins each sheet on the main thread, whose run loop must keep running until the sheet completes, so the same worker-thread pattern applies.

Linux presentation needs an xdg-desktop-portal backend in the test desktop and remains unverified.

## References

- [Tauri dialog APIs](https://v2.tauri.app/plugin/dialog/)
- [Electron dialog APIs](https://www.electronjs.org/docs/latest/api/dialog)
- [rfd platform and threading requirements](https://docs.rs/rfd/latest/rfd/)
- [Browser picker API](https://developer.mozilla.org/en-US/docs/Web/API/Window/showOpenFilePicker)
