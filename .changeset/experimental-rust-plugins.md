---
leptoskit_bridge: minor
leptoskit_bridge_ffi: minor
leptoskit_config: minor
leptoskit_cli: minor
leptoskit_plugin_clipboard: minor
leptoskit_plugin_haptics: minor
---

Add experimental pure-Rust native plugin support (`native.experimental-rust-plugins = true`) with shared `AppContext` and `PlatformContext` host handles (`JavaVM`/`Context` over JNI on Android and main-thread `NativePluginContext` owner resolution via `objc2` on iOS and macOS), UI-thread Rust plugin dispatch (`rust-main-thread = true` / `App::install_main`), and experimental pure-Rust `clipboard` and `haptics` backends across iOS, macOS, and Android.
