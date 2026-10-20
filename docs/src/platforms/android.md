# Android

The Android runtime probe exercises the shared Rust layout, renderer, wasm runtime, and update client. [Generated native apps](host-projects.md) render the guest's tree with Android views, and can replace that guest through [code push](host-projects.md#code-push).

## Architecture

```
Android views (android.view.View hierarchy)
   ▲ mutations (create / update props / insert / layout)
NativeBackend (the platform half, implemented over the View system)
   ▲
leptoskit_native renderer                              ─┐
leptoskit_yoga layout                                    ├─ shared, pure Rust
leptoskit_wasm runtime (wasmi interpreter)               │
leptoskit_codepush client                               ─┘
   ▲ your app's wasm bundle
```

`leptoskit_yoga` is an in-house Rust layout engine. Native hosts share that implementation. The Leptos web adapter emits CSS and uses browser layout instead.

## Building for the emulator

Cross-compile with an NDK toolchain clang as the linker:

```bash
rustup target add aarch64-linux-android
export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER=\
"$HOME/Library/Android/sdk/ndk/<version>/toolchains/llvm/prebuilt/<host>/bin/aarch64-linux-android24-clang"
cargo build --release --package platform_probe --target aarch64-linux-android
```

No JNI, no Gradle, no AndroidX dependency — the stack is pure Rust linking against bionic.

## Validating on the emulator

```bash
emulator -avd Pixel_7_Pro_API_36 &          # or boot from your IDE
adb push target/aarch64-linux-android/release/platform_probe /data/local/tmp/
adb shell chmod +x /data/local/tmp/platform_probe
adb shell /data/local/tmp/platform_probe
```

Expected output, on device:

```
PASS yoga:flexbox
PASS yoga:grid
PASS native:diff-and-events
PASS wasm:abi-runtime
PASS codepush:delta-update
platform probe: all probes passed on android
```

The delta-update probe runs the full client path on the emulator OS: check, delta download, apply, hash verify, stage.

## Shipping

Same model as iOS. The store artifact carries the shell: an APK or AAB with a small Kotlin activity that starts the runtime. Interface versions ship over the air through code push, with binary deltas. Updates are downloaded, tried, and confirmed exactly as on iOS, and CI runs the same journey on an emulator. New native capabilities, such as plugins, permissions, or new `Custom` widget kinds, go in a store release with a new `native.update_contract`. Everything else rides the channel.

```bash
leptoskit native bundle --out dist
leptoskit codepush release --channel production
```
