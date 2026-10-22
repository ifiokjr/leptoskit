# Windows and Linux

Windows and Linux render the shared component tree with their own native toolkits, through the same `NativeBackend` contract AppKit uses: [`leptoskit_windows`](https://github.com/ifiokjr/leptoskit/tree/main/crates/leptoskit_windows) commits Win32 windows and native common controls, and [`leptoskit_gtk`](https://github.com/ifiokjr/leptoskit/tree/main/crates/leptoskit_gtk) commits GTK 4 widgets. Neither embeds a browser.

## The host contract, per platform

| Platform | Crate               | Widget layer                   | Backend maps mutations to                                   |
| -------- | ------------------- | ------------------------------ | ----------------------------------------------------------- |
| macOS    | `leptoskit_appkit`  | AppKit                         | `NSView` hierarchy                                          |
| Windows  | `leptoskit_windows` | Win32 and common controls (v6) | HWNDs: `EDIT`, `BUTTON`, trackbars, and composition windows |
| Linux    | `leptoskit_gtk`     | GTK 4                          | `GtkWidget` hierarchy, with Pango text                      |

The contract is [`NativeBackend`](https://github.com/ifiokjr/leptoskit/tree/main/crates/leptoskit_native): create, update props, update layout, insert child, remove child, and delete, applied in batch order, plus a `TextMeasurer` so layout matches the platform's font metrics. Everything above it (the renderer, layout, wasm runtime, and update client) is the same pure Rust on every target.

## Linux

`leptoskit_gtk` needs GTK 4.6 or newer. Initialize GTK on the application thread, construct `GtkBackend`, and use it in a `NativeSurface`. `open_window` hosts a committed root, and `run_application` runs the GTK event loop until the windows close.

On Debian or Ubuntu, install `libgtk-4-dev` and `pkg-config` to build it. The widget harness opens real GTK windows, sends pointer and keyboard input, and checks accessibility values and rendered pixels:

```sh
GDK_BACKEND=x11 GSK_RENDERER=cairo dbus-run-session -- xvfb-run -a \
  cargo test -p leptoskit_gtk --test widgets
```

## Windows

`leptoskit_windows` owns its HWNDs on one UI thread. An application must embed the crate's `app.manifest` in its executable to enable version 6 common controls and layered child windows on Windows 10 and later. Create a `WindowsWindow`, set the backend's DPI to `window.dpi()`, and then construct the `NativeSurface`; `examples/probe.rs` in the crate is a complete host.

Run the native checks on Windows with an interactive desktop:

```powershell
cargo test -p leptoskit_windows --test widgets -- --test-threads=1
cargo run -p leptoskit_windows --example probe -- native-artifacts/windows
```

## Why the port surface stays small

The stack below the widgets is pure Rust:

- `leptoskit_yoga` has no system layout dependency, so the same flexbox and grid numbers come out on every OS.
- `leptoskit_wasm` runs `wasmi`, a pure Rust interpreter with no JIT, so it needs no JIT entitlements and compiles anywhere Rust does.
- `leptoskit_codepush` speaks HTTP over each platform's stack; the protocol and verification are pure Rust.
- `leptoskit_native` has no platform types in the renderer or mutation model; only a backend does.

That is the lesson of React Native's Windows and macOS ports: when the shared layer is genuinely shared, a new desktop target is one backend crate, not a port.

## Status

Both backends render every built-in primitive and run in CI: GTK under Xvfb on Linux, Win32 on a Windows runner. They are younger than AppKit, so expect gaps in controls and styling. The [testing guide](../guide/08-testing.md) shows how to drive them with the native UI driver.
