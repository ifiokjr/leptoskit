<h1 align="center">leptoskit_plugin_clipboard</h1>

<p align="center">
  <strong>leptoskit_plugin_clipboard</strong> is the leptoskit plugin for system text clipboard access - read, write, and clear - on iOS, Android, macOS, Windows, Linux, and in the browser.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://docs.rs/leptoskit_plugin_clipboard"><strong>Docs</strong></a> ·
  <a href="https://github.com/ifiokjr/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<p align="center">
  <a href="https://crates.io/crates/leptoskit_plugin_clipboard"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_plugin_clipboard.svg"></a>
  <a href="https://docs.rs/leptoskit_plugin_clipboard"><img alt="docs.rs" src="https://docs.rs/leptoskit_plugin_clipboard/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<br>

## Overview

System text clipboard access: read, write, and clear plain text on iOS, Android, macOS, Windows, Linux, and in the browser. Only text is supported; images and rich content are not.

Add the crate as a direct app dependency and select `"clipboard"` in `native.plugins`. The generator discovers `ClipboardPlugin` from the crate's Cargo package metadata (a Swift class on Apple platforms, a Kotlin class on Android). No permission prompt is needed.

Guest applications enable the `guest` feature and construct the client once, which caches the plugin's numeric ID.

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

## How it fits

Add `leptoskit_plugin_clipboard` as a direct dependency and select the `clipboard` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::Clipboard` rather than resolving the plugin on every call. Only plain text is supported; browser operations need a secure context and may need a user gesture.

## Use it

```rust,ignore
let clipboard = leptoskit_plugin_clipboard::guest::Clipboard::new()?;
clipboard.write_text("copied from leptoskit").await?;
let text = clipboard.read_text().await?; // empty when nothing textual is copied
clipboard.clear().await?;
```

## Learn more

- [Crate documentation](https://docs.rs/leptoskit_plugin_clipboard)
- [leptoskit documentation](https://ifiokjr.github.io/leptoskit/)
- [Repository](https://github.com/ifiokjr/leptoskit)
- [Plugin inventory](https://ifiokjr.github.io/leptoskit/platforms/plugin-inventory.html)
