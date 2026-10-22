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

`examples/native_fixture.rs` is a Win32, AppKit, or GTK4 window whose buttons present this plugin's file chooser and message box. The `native UI` workflow's Windows, macOS, and Linux jobs drive it with `leptoskit_testing`'s `dialog_plugin_journey` example, through NovaWindows, Mac2, and KDE's AT-SPI WebDriver respectively. The journey answers both dialogs with the operating system's own Open, Cancel, OK, and Ok buttons — zenity labels its confirmation "Ok" on Linux — asserts the result returned to the app, and keeps screenshots of every dialog. A worker thread awaits each dialog while the window keeps pumping messages: Windows enables and disables a dialog's owner synchronously, so awaiting on the owner's thread would deadlock. On macOS, rfd begins each sheet on the main thread, whose run loop must keep running until the sheet completes. On Linux, rfd exports the GTK window's Wayland surface through xdg-foreign to parent the portal's dialog and waits off the main loop, which keeps answering the driver's accessibility reads.

The Linux test desktop runs xdg-desktop-portal with its KDE backend, which renders the real KDE file chooser; the journey types the suggested `evidence.txt` to select it, because the portal preselects nothing. rfd presents the message box through zenity, which the desktop also runs. The AT-SPI driver never performs GTK's own `click` action and GTK's bridge reports every control at the screen's origin, so the Linux desktop is driven with keys: the fixture starts with its first button focused, Space presents the chooser, and zenity answers Enter with its default and Escape with cancellation.

## References

- [Tauri dialog APIs](https://v2.tauri.app/plugin/dialog/)
- [Electron dialog APIs](https://www.electronjs.org/docs/latest/api/dialog)
- [rfd platform and threading requirements](https://docs.rs/rfd/latest/rfd/)
- [Browser picker API](https://developer.mozilla.org/en-US/docs/Web/API/Window/showOpenFilePicker)
