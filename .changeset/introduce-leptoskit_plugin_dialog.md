---
leptoskit_plugin_dialog: major
---

# leptoskit_plugin_dialog: native dialogs

`leptoskit_plugin_dialog` is the leptoskit library for asynchronous native file, directory, save, and message dialogs on macOS, Windows, and Linux, plus browser file selection.

Rust desktop hosts and browser apps call it directly: it has no `native.plugins` entry, no guest API, and no automatic registration, because each dialog needs a running event loop and a live parent window. On desktop, `desktop::FileDialog` and `desktop::MessageDialog` borrow that parent window until the dialog closes, and Linux needs an XDG desktop portal and Zenity. In the browser, `web::open_files` uses the File System Access API from a user action on HTTPS, and `web::read_file` reads a selection under an explicit byte limit; browser save, directory, and message dialogs, and mobile dialogs, are not provided.
