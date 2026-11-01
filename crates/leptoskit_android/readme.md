<h1 align="center">leptoskit_android</h1>

<p align="center">
  <strong>leptoskit_android</strong> is the Android Views backend: it applies the shared mutation stream to real Android views, with Yoga owning exact outer frames and Android owning each control's interior.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://docs.rs/leptoskit_android"><strong>Docs</strong></a> ·
  <a href="https://github.com/ifiokjr/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<p align="center">
  <a href="https://crates.io/crates/leptoskit_android"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_android.svg"></a>
  <a href="https://docs.rs/leptoskit_android"><img alt="docs.rs" src="https://docs.rs/leptoskit_android/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<br>

## Overview

Android Views backend for `NativeSurface`. Compile `android/WidgetHost.kt` into an Android application and construct `AndroidBackend::new` from the host object on the UI thread. Set `android:supportsRtl="true"` on the manifest's application so Android honors the shared writing direction. `WidgetHost` accepts the view's `Context`. Keep the Rust surface alive for the Activity lifetime. Mount the root with `WidgetHost.mount(rootId, frameLayout)` so fixed dimensions and margins retain their Yoga allocation. Forward `onStart` and `onStop` to `WidgetHost.setForeground`, and drop the surface in `onDestroy`.

Every built-in shared primitive renders a real Android view. Yoga owns exact outer frames in logical dp; Android controls own their interior chrome. Text uses `TextPaint` and `StaticLayout` with matching drawing fonts. Images accept local paths, file URLs, and explicit `android.resource` URIs. Network loading is not implicit. The Kotlin host is independent of the native plugin ABI.

Native callbacks propose typed events to the single backend listener. Forward them to `NativeSurface::dispatch_event` and commit application state. The host restores the latest committed values after rejected actions and reentrant updates/removals. The backend and views stay on the UI thread; only the pure text measurer may cross threads.

Run `cargo run -p leptoskit_android --example verify_device` with `ANDROID_SERIAL` naming a dedicated emulator. The Rust verifier builds and runs the real test Activity on macOS or Linux. It preserves an explicit `JAVA_HOME`, uses the SDK's NDK, and writes APK, assertion logs, and a screenshot under `LEPTOSKIT_ANDROID_ARTIFACTS`. `--coverage` instruments the library and also pulls its LLVM profile as `coverage.profraw`. The packaging, signing, and install steps live in the workspace's `setup/android_apk` crate, which the gallery's `verify_mobile` shares.

The verifier requires SDK API 35, build-tools 36.0.0, NDK 27.1.12297006, `kotlinc`, Java 17 or newer, `curl`, and `zip`. The Activity requires Android 9 (API 28). Both `arm64-v8a` and `x86_64` test ABIs are supported. The verifier pins Kotlin stdlib 2.1.21 by SHA-256 to match D8 metadata support, and aligns the native library and APK for 16 KiB pages. It checks actual Android controls, controlled rejection and replacement, native accessibility, logical viewport changes, RTL text, background state, scroll extents, and animation teardown. `widgets.png` captures the hardware-rendered Activity with PixelCopy.

## How it fits

Compile `android/WidgetHost.kt` into an Android app and construct `AndroidBackend::new` on the UI thread. Every built-in primitive renders a real view; callbacks propose typed events that the app commits through `NativeSurface`.

## Use it

```rust,ignore
use leptoskit_android::AndroidBackend;
use leptoskit_native::NativeSurface;

let backend = AndroidBackend::new(host);
let surface = NativeSurface::new(backend);
```

## Learn more

- [Crate documentation](https://docs.rs/leptoskit_android)
- [leptoskit documentation](https://ifiokjr.github.io/leptoskit/)
- [Repository](https://github.com/ifiokjr/leptoskit)
- [Android](https://ifiokjr.github.io/leptoskit/platforms/android.html)
