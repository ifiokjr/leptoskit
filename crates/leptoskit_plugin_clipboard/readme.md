# leptoskit_plugin_clipboard

System text clipboard access: read, write, and clear plain text on iOS, Android, macOS, Windows, Linux, and in the browser. Only text is supported; images and rich content are not.

Add the crate as a direct app dependency and select `"clipboard"` in `native.plugins`. The generator discovers `ClipboardPlugin` from the crate's Cargo package metadata (a Swift class on Apple platforms, a Kotlin class on Android). No permission prompt is needed.

Guest applications enable the `guest` feature and construct the client once, which caches the plugin's numeric ID:

```rust,ignore
let clipboard = leptoskit_plugin_clipboard::guest::Clipboard::new()?;
clipboard.write_text("copied from leptoskit").await?;
let text = clipboard.read_text().await?; // empty when nothing textual is copied
clipboard.clear().await?;
```

| Platform              | Implementation                                                                                                                                 |
| --------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------- |
| iOS                   | `UIPasteboard`                                                                                                                                 |
| macOS Swift host      | `NSPasteboard`                                                                                                                                 |
| Android               | `ClipboardManager`; Android itself enforces foreground and activity restrictions on background reads                                           |
| macOS, Windows, Linux | `Clipboard`, a Rust `Plugin` over [arboard](https://crates.io/crates/arboard). Keep it alive for as long as the contents should stay available |
| Browser               | `web::read_text`, `web::write_text`, `web::clear` over the Async Clipboard API, which needs a secure context and usually a user gesture        |

On X11 the clipboard owner must stay alive, which is why the desktop `Clipboard` is a long-lived value rather than a function. Reading when no text is available returns an empty string, not an error. The browser has no way to clear, so `clear` writes an empty string. Browser permission refusals return `PermissionDenied`.

With the `hooks` feature, `signals::use_clipboard` mirrors the clipboard in a reactive `text` signal and returns `read`, `write`, and `clear` actions. The signal changes only after the platform accepts the operation. The actions are futures: nothing is spawned for you, so run them on your host's executor.

The binary contract: method `0` reads (empty payload, returns UTF-8), method `1` writes (the payload is the raw UTF-8 text), and method `2` clears (empty payload). Unknown methods return `Unavailable`; a payload where none is expected, or invalid UTF-8 on write, returns `InvalidArgument`.

The desktop test uses the real system clipboard (CI runs Linux under Xvfb) and restores whatever text was there. Apple behavior is tested by `apple-tests`, run with `devenv shell test:plugins:apple`.
