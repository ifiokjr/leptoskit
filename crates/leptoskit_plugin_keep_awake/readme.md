<h1 align="center">leptoskit_plugin_keep_awake</h1>

<p align="center">
  <strong>leptoskit_plugin_keep_awake</strong> is the leptoskit plugin for scoped display wake locks with cancellation-safe ownership.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://docs.rs/leptoskit_plugin_keep_awake"><strong>Docs</strong></a> ·
  <a href="https://github.com/ifiokjr/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<p align="center">
  <a href="https://crates.io/crates/leptoskit_plugin_keep_awake"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_plugin_keep_awake.svg"></a>
  <a href="https://docs.rs/leptoskit_plugin_keep_awake"><img alt="docs.rs" src="https://docs.rs/leptoskit_plugin_keep_awake/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<br>

## Overview

Scoped display wake locks with cancellation-safe ownership. This plugin prevents display idle sleep; it does not promise unrestricted background execution or override explicit OS power policy.

For an interpreted Wasm app, enable the crate's `guest` feature, select `keep-awake` in `native.plugins`, and regenerate the host. `guest::KeepAwake::new()?.hold()?` returns a holding future. Retain it and poll it alongside application work, for example with `futures::future::select`. It never completes successfully: platform failure or lifecycle termination returns an error, and dropping the future cancels its native request and releases its reference. Pending is not an acquisition acknowledgement. This contract avoids a successful acquire response racing cancellation and leaving an orphaned token.

Each native request holds one reference, with at most 64 simultaneous requests per instance. iOS preserves `UIApplication.isIdleTimerDisabled` before the first request and restores it after the last; entering the background cancels all leases. Android uses the activity's `FLAG_KEEP_SCREEN_ON`, preserving an existing flag and restoring it when the final lease ends. It requires the activity's content to be visible, not focused, so a lease can start at launch or behind a dialog. It cancels leases when that activity stops or is destroyed. Plugin close cancels all pending requests and restores ownership exactly once. Do not independently change these same flags from another plugin instance while leases exist.

Desktop Rust callers use `desktop::Lease::acquire(application, reason)`. Leases and clones on the same thread share a real OS assertion, released after the final owner drops. They intentionally cannot cross threads because Windows execution-state restoration is thread-specific. The first live owner supplies the reason. The implementation calls IOKit assertions on macOS, Windows execution state, and the desktop ScreenSaver D-Bus inhibitor on Linux. Windows preserves the thread's previous requirements; do not independently change execution state while a lease exists. Linux needs the corresponding session service; acquisition failure is returned. Its private D-Bus connection owns the inhibitor, and disconnecting releases it according to the [idle-inhibit specification](https://specifications.freedesktop.org/idle-inhibit/latest/). Desktop acquisition can block on an OS service and belongs off the UI thread. macOS's Swift host also supports the pending HOLD contract through `ProcessInfo` display activities.

Browser `web::hold().await` uses the Screen Wake Lock API in a secure, visible context. Retain/poll that future alongside work. Cancellation releases the sentinel even if acquisition resolves after cancellation. Browser revocation or visibility loss ends the future with `Cancelled`; reacquisition is always explicit. Unsupported browsers return `Unavailable`, and permission/policy rejection is preserved. Cleanup errors that cannot be returned from Drop are reported to the browser console.

Wire method `0` accepts only empty bytes and remains pending for the entire lease. Successful completion is a protocol error. A dropped guest future uses the bridge's existing cancellation path, so cleanup requires no separate release message or unpolled fire-and-forget reply.

References: [Expo KeepAwake](https://docs.expo.dev/versions/latest/sdk/keep-awake/), [Screen Wake Lock API](https://developer.mozilla.org/en-US/docs/Web/API/Screen_Wake_Lock_API).

## How it fits

Add `leptoskit_plugin_keep_awake` as a direct dependency and select the `keep-awake` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::KeepAwake` rather than resolving the plugin on every call. iOS preserves and restores the idle timer, Android uses the activity's keep-screen-on flag, desktop uses OS assertions, and the browser uses the Screen Wake Lock API; none of them promise background execution.

## Use it

```toml
# In a native host, select `keep-awake` in `native.plugins`.
[dependencies]
leptoskit_plugin_keep_awake = { version = "0.1", features = ["guest"] }
```

## Learn more

- [Crate documentation](https://docs.rs/leptoskit_plugin_keep_awake)
- [leptoskit documentation](https://ifiokjr.github.io/leptoskit/)
- [Repository](https://github.com/ifiokjr/leptoskit)
- [Plugin inventory](https://ifiokjr.github.io/leptoskit/platforms/plugin-inventory.html)
