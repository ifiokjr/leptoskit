# Native and web gallery

Seven screens built from the shared leptoskit primitives and Tailwind classes. Every host renders the same screen functions: AppKit on macOS, GTK 4 on Linux, Win32 on Windows, UIKit on iOS, Android Views on Android, and Leptos in the browser. Images are generated locally and need no network access. Run the commands below from the repository root inside `devenv shell`.

## Desktop

```sh
cargo run -p gallery -- list
cargo run -p gallery -- window counter
cargo run -p gallery -- snapshot target/gallery/desktop
```

`snapshot` renders each screen with the platform's own widgets and renderer and writes `<screen>.png`. Linux needs GTK 4.6 or newer and a display; under CI, wrap the command in `xvfb-run` with `GDK_BACKEND=x11`. Windows MSVC builds embed the Win32 backend's manifest for common controls 6 and per-monitor DPI. Resizing a window updates the layout viewport, and a monitor scale change remeasures text.

## iOS simulator

```sh
cargo run -p gallery_verify --example verify_mobile -- ios --device "$SIMULATOR_UDID"
```

This builds `gallery-ios` for the simulator matching this Mac, packages and ad-hoc signs `Gallery.app`, then launches every screen. Each launch inspects the mounted UIKit views, presses the counter's native button, and must report `VERIFIED`. Instead of `--device`, pass `--runtime <id> --device-type <id>` to run on a simulator created and deleted for this run. The app, reports, and screenshots land in `target/gallery/ios`. To open one screen yourself:

```sh
xcrun simctl install "$SIMULATOR_UDID" target/gallery/ios/Gallery.app
xcrun simctl launch "$SIMULATOR_UDID" dev.leptoskit.gallery components
```

Each window scene owns its UIKit surface; rotation resizes it, and controls reconcile the shared screen. Device builds need signing through Xcode.

## Android

Install Android SDK platform 35, build tools 36, NDK 27.1.12297006, Java, `kotlinc`, and the Rust Android target, and set `ANDROID_HOME`. Then point the verifier at a dedicated emulator or device:

```sh
ANDROID_SERIAL=emulator-5554 ANDROID_ABI=arm64-v8a cargo run -p gallery_verify --example verify_mobile -- android
```

This cross-compiles the gallery into `libgallery.so`, packages a signed `gallery.apk` with 16 KiB page alignment, installs it, and runs its instrumentation. The journey opens every screen in `GalleryActivity`, taps the real counter button, rotates the Activity and checks that the button and the count survive, scrolls the phone-sized component studio, and taps its native pressable. It must report `VERIFIED`. The APK, instrumentation output, logcat, and screenshots land in `target/gallery/android`. Use `ANDROID_ABI=x86_64` for an Intel emulator; `ANDROID_NDK_HOME`, `ANDROID_NDK_VERSION`, `ANDROID_BUILD_TOOLS_VERSION`, and `ANDROID_API_LEVEL` override the defaults. To open one screen yourself:

```sh
adb install -r target/gallery/android/gallery.apk
adb shell am start -n dev.leptoskit.gallery/.GalleryActivity --es screen components
```

The Activity owns its Rust session: resizes use logical dp, rotation keeps the live widgets, backgrounding pauses native animations, and destruction drops the widgets on the UI thread. Its JNI entrypoints are independent of the plugin bridge.

## Browser and tests

```sh
cd examples/gallery
trunk serve
```

One conformance scenario runs on every host: each screen renders with that host's widgets, and pressing the counter's native button updates the shared state without replacing the control.

- `cargo test -p gallery` runs it on the desktop's backend (`tests/desktop.rs` and `tests/cli.rs`). On Linux it needs a display, for example `dbus-run-session -- xvfb-run -a env GDK_BACKEND=x11 cargo test -p gallery`. The same command also lays every screen out headlessly at its own size and at phone size.
- `test:browser` runs it in Chrome (`tests/web.rs`).
- `verify_mobile` runs it on iOS and Android, as described above.
