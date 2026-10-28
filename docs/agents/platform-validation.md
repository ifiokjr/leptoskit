# Platform validation

How the repository proves that each platform works: the runtime probe, the widget verifiers, the native fixtures, and what CI runs. None of this is needed to build an app with leptoskit, so it lives here rather than on the public site.

## What CI runs per platform

| Platform        | CI job                                  | What it exercises                                                                                                                                                                                                             |
| --------------- | --------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| macOS (AppKit)  | `test (macos-latest)` in `ci.yml`       | Workspace tests on a real macOS runner, the `leptoskit_appkit` suite with offscreen PNG snapshots, and a generated macOS app build. Not part of the coverage gate.                                                            |
| iOS (UIKit)     | `ios simulator e2e`                     | Boots a simulator and runs the instrumented `native_probe` through `verify_simulator`, with coverage, plus the generated app's UI test and the code push journey.                                                             |
| Android         | `android build and widget tests`        | An API 35 x86_64 emulator running `verify_device`, the widget suite, `verify_mobile` gallery screenshots, Gradle instrumented tests, and a launch of the R8-minified release build of the file-routes example, with coverage. |
| Linux (GTK 4)   | `coverage (ubuntu-latest)`              | The real GTK widget suite under Xvfb with the cairo renderer and AT-SPI.                                                                                                                                                      |
| Windows (Win32) | `test (windows-latest)`                 | `leptoskit_windows --test widgets` against real Win32 windows, the workspace tests, a generated-app build, and the widget journey. Not part of the coverage gate.                                                             |
| HarmonyOS       | `harmonyos.yml`, separate from `ci.yml` | Deno render-model tests, host contracts, and the Wasm counter, then compiles the app and its instrumentation HAPs for both ABIs. Nothing runs on an emulator or device. See [HarmonyOS](harmonyos.md).                        |
| System plugins  | `native-ui.yml`                         | Dialog and permission plugins on all five desktop and mobile platforms. It does not test the widget backends.                                                                                                                 |

The coverage gate merges only the Linux, browser, iOS, and Android reports; see [CONTRIBUTING.md](../../CONTRIBUTING.md#coverage).

## The platform probe

The [`platform_probe`](../../examples/platform_probe) binary exercises layout, renderer diffing, the wasm ABI runtime, and a code push delta update over HTTP. Its suite runs on the host, the iOS simulator, and an Android emulator:

```bash
devenv shell test:all                      # includes the desktop probe
ios_sim_run                                # the iOS simulator
android_emulator_run                       # an Android device or emulator
```

The probes verify these runtime contracts. The widget backends have verifiers of their own, which commit real widgets on each platform. Neither shows complete CSS support.

Expected output, on a simulator or emulator:

```
PASS yoga:flexbox
PASS yoga:grid
PASS native:diff-and-events
PASS wasm:abi-runtime
PASS codepush:delta-update
platform probe: all probes passed on ios
```

The `codepush:delta-update` probe ingests two releases into a real store, serves them through the real axum router, and updates a client over the wire with a binary delta that hash-verifies before staging, all on the simulator or emulator OS.

On macOS the desktop run is part of `devenv shell test:all`: the same five probes execute as a native arm64 process, so every layer below the widgets runs on the desktop OS, and the widget layer is the AppKit backend above it.

### iOS simulator by hand

```bash
rustup target add aarch64-apple-ios-sim
cargo build --release --package platform_probe --target aarch64-apple-ios-sim
xcrun simctl boot "iPhone 17 Pro"   # or any available device
xcrun simctl install booted target/platform_probe.app
xcrun simctl launch --console-pty booted dev.leptoskit.platform-probe
```

Link with Xcode's clang (the `xcrun --find clang` toolchain), not a nix-wrapped compiler. [`setup/scripts/ios_probe_run.sh`](../../setup/scripts/ios_probe_run.sh) and the CI workflow have the exact environment.

### Android emulator by hand

Cross-compile with an NDK toolchain clang as the linker:

```bash
rustup target add aarch64-linux-android
export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER=\
"$HOME/Library/Android/sdk/ndk/<version>/toolchains/llvm/prebuilt/<host>/bin/aarch64-linux-android24-clang"
cargo build --release --package platform_probe --target aarch64-linux-android
emulator -avd Pixel_7_Pro_API_36 &          # or boot from your IDE
adb push target/aarch64-linux-android/release/platform_probe /data/local/tmp/
adb shell chmod +x /data/local/tmp/platform_probe
adb shell /data/local/tmp/platform_probe
```

The probe binary needs no JNI, Gradle, or AndroidX: it is pure Rust linking against bionic. Real apps render through the Kotlin `WidgetHost` and build with Gradle and the NDK.

## The portable widget journey

The SDK's `widget_suite` example is the journey CI runs on every OS through `leptoskit test run`:

```sh
cargo build -p leptoskit_testing --example widget_suite
leptoskit test run --binary target/debug/examples/widget_suite
cargo build -p leptoskit_testing --example widget_suite --target aarch64-apple-ios-sim
cargo build -p leptoskit_testing --example widget_suite --target aarch64-linux-android
```

The AppKit suite (`crates/leptoskit_appkit/tests/appkit.rs`) must run on the main thread. It checks the actual `NSTextField` content after an interaction and renders a PNG; set `LEPTOSKIT_TEST_ARTIFACTS` to keep it.

## Native fixtures

The SDK's `native_plugin_suite` example prepares an extended Android fixture with selected optional dependencies. It includes synthetic activity-result broker tests, orientation cancellation/close restoration and bounded light-sensor acquisition. Pedometer registration compiles without requesting real activity permission or history. Document import bounds and cancellation are tested with synthetic streams/files; these fixtures do not validate interactive native picker UI or access user documents.

The repository also includes a native async fixture. Start its loopback server in one terminal, then generate and build its app in another:

```sh
cargo run -p leptoskit_cli --example async_plugin_server
cargo build -p leptoskit_cli --bin leptoskit
cargo run -p leptoskit_cli --example async_plugin_suite -- \
  --cli target/debug/leptoskit --destination target/async-app --target macos
cd target/async-app
leptoskit native build macos
```

Its **Check files** action also checks HTTP binary data, denied origins, redirects, response limits, cancellation, and WebSocket text/binary messages and close. For an arm64 Android emulator, generate with `--target android --host 10.0.2.2` (use `android-x86_64` for x86-64 emulators) so its explicit host scopes point to the development machine. The fixture server binds only loopback; it is test tooling, not an app service.

## Code push journey

`cargo run -p leptoskit_cli --example mobile_codepush -- --platform ios|android --device <id> --sdk-path . --cli target/debug/leptoskit` runs the full journey against a booted simulator or emulator, and CI runs it on both platforms. It scaffolds an app, builds four signed guests from one source, and serves them from a local registry. Then it taps a counter that steps by one, downloads and activates a replacement that steps by ten, and relaunches offline. Then it activates a guest that traps on startup and checks that the working guest is still interactive, both straight away and after a relaunch. Finally it installs a second good update after that rollback, and checks that it survives an offline relaunch.

## `leptoskit dev` reload

`crates/leptoskit_cli/tests/dev_reload.rs` checks that `leptoskit dev` reloads a real app: it scaffolds a project against this checkout, runs `leptoskit dev --platform <platform>`, edits the home screen's heading, and waits until every app `dev` launched logs `leptoskit dev: reloaded`. The shells log that line only once the new guest's native views have rendered. On Android the test also reads each device's view hierarchy with `uiautomator dump` and requires the edited text on screen. The tests need the platform toolchain and a device, so they are ignored by default and CI does not run them yet:

```sh
cargo nextest run -p leptoskit_cli --test dev_reload --run-ignored ignored-only
cargo nextest run -p leptoskit_cli --test dev_reload --run-ignored ignored-only -E 'test(android)'
```

iOS uses every booted iPhone simulator, and boots the newest one when none is booted. Android uses every device `adb devices` lists, phones and emulators alike, and boots the first emulator when none is attached. Each run keeps its project and the full `dev` log under `target/dev-reload/<platform>-<time>/`.
