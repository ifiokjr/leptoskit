# Windows and Linux

Windows and Linux hosts complete the desktop matrix. The architecture is identical to macOS — a wasm module driving native widgets through a `NativeBackend` — with the backend implemented over each platform's widget toolkit, the way React Native Windows wraps XAML/Composition and React Native Linux wraps GTK.

## The host contract, per platform

| Platform | Widget layer          | Backend maps mutations to    |
| -------- | --------------------- | ---------------------------- |
| macOS    | AppKit                | `NSView` hierarchy           |
| Windows  | WinUI 3 / Composition | `Microsoft.UI.Xaml` controls |
| Linux    | GTK 4                 | `GtkWidget` hierarchy        |

The contract is [`NativeBackend`](../../crates/leptoskit_native): create, update props, update layout, insert child, remove child, delete — applied in batch order, plus a `TextMeasurer` so layout matches the platform's font metrics. Everything above it (renderer, layout, wasm runtime, update client) is the same pure Rust that already passes the probe suite on macOS, iOS, and Android, and compiles for `x86_64-pc-windows-msvc` and the Linux targets out of the box because it has no platform dependencies.

## Why the port surface stays small

The stack below the widgets is pure Rust:

- `leptoskit_yoga` — no system layout dependency; the same flexbox and grid numbers on every OS.
- `leptoskit_wasm` — `wasmi` is a pure Rust interpreter; no JIT, so no platform JIT entitlements, and it compiles anywhere Rust does.
- `leptoskit_codepush` — HTTP over each platform's stack; the protocol and verification are pure Rust.
- `leptoskit_native` — the renderer and mutation model have no platform types; only the backend implementation does.

That is the RN Windows/Linux lesson applied: the shared layer is genuinely shared, and a new desktop target is one backend crate, not a port.

## Status

The desktop contract and shared stack are validated continuously on macOS (the desktop probe in `test:all`). Windows and Linux hosts implement the same `NativeBackend` trait over WinUI and GTK; until each lands, the honest claim is exactly this: everything below the widgets runs on every target, and the widget layer per OS is the remaining work — one crate each, with the probe suite as the acceptance test.
