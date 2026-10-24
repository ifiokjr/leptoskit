# Testing

Leptoskit separates widget behavior from operating-system integration. Most component behavior belongs in fast widget tests; device tests verify that the same code executes in the target environment. Native input routing and system dialogs need platform automation in addition to either of those.

## Native widget tests

Add `leptoskit_testing` as a development dependency alongside `leptoskit_native`:

```rust
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use leptoskit_native::Element;
use leptoskit_testing::{Finder, WidgetTester};

let count = Arc::new(AtomicUsize::new(0));
let mut tester = WidgetTester::new(move || {
    let action = Arc::clone(&count);
    Element::view()
        .child(Element::text(format!("Count: {}", count.load(Ordering::SeqCst))))
        .child(Element::button("Add").on_press(move || {
            action.fetch_add(1, Ordering::SeqCst);
        }))
}, 390.0, 844.0)?;

tester.tap(&Finder::text("Add"))?;
tester.find(&Finder::text("Count: 1"))?;
# Ok::<(), leptoskit_testing::TestError>(())
```

Prefer `Finder::aria_role(Role::Button, "Add")` — like Testing Library's `getByRole`, counting the role a widget kind implies — or accessibility labels where they distinguish controls. `WidgetTester::semantics()` prints the semantics tree a screen reader would expose, for accessibility assertions. `Finder::test_id("counter")` reads the string property `extra["test_id"]`; use IDs only when semantics do not identify the widget.

Actions re-query the current tree, reject ambiguous and disabled controls, dispatch through real handlers, and commit one new frame. `enter_text` requires a `TextInput`; `scroll` requires a `ScrollView` and calls the component's handler. Scrolling does not emulate a native view's internal scroll offset. `find_all` returns owned observations; they cannot be used as stale action targets.

`pump_until(&finder, max_pumps)` checks immediately and then advances at most that many render passes. Async test code must advance its runtime before calling `pump`. No arbitrary sleeps or unbounded “settle” loop are involved.

Errors carry the tree, labels, values, frames, and visibility/enabled states. `resize` re-lays out for orientation and viewport tests. Dropping the tester drops the component closure, surface, and handlers.

`WidgetTester::with_backend` accepts a real backend. AppKit tests must run on the main thread; see `crates/leptoskit_appkit/tests/appkit.rs`. That suite checks the actual `NSTextField` content after an interaction and renders a PNG. Set `LEPTOSKIT_TEST_ARTIFACTS` to retain it. Headless metrics approximate fonts; use a platform backend for native text sizing and screenshots.

## Browser component tests

Browser components continue to use `leptos_testing_library::render_for_test` and `wasm-bindgen-test`. They mount real Leptos components into a real browser and advance reactive updates with `leptos::task::tick().await`.

```sh
leptoskit test web --package leptos_testing_library --browser chrome
leptoskit test web --package leptos_testing_library --browser firefox
```

Install the `wasm32-unknown-unknown` target, the matching wasm-bindgen test runner, and the requested browser and WebDriver. The workspace's devenv provides the pinned tooling. `CHROMEDRIVER` or `GECKODRIVER` can specify an absolute driver path.

## Run the same widget journey on each OS

The runner accepts an already compiled executable. It uses its exit status, including Rust libtest executables, as the verdict. It does not infer success from an empty log or a printed keyword. Application bundles, APKs, and production app UI automation are different artifacts and are not accepted as test executables.

```sh
cargo build -p leptoskit_testing --example widget_suite
leptoskit test run --binary target/debug/examples/widget_suite
```

On Windows, build on the Windows host and pass `target/debug/examples/widget_suite.exe`. CI allocates Windows, Linux, and macOS hosts and runs the same tests on each. The CLI does not install or license a Windows VM on another OS.

### iOS Simulator

Use Xcode on macOS and a simulator runtime already installed on the machine:

```sh
xcrun simctl list devices available
cargo build -p leptoskit_testing --example widget_suite --target aarch64-apple-ios-sim
leptoskit test run --platform ios --device SIMULATOR-UDID \
  --binary target/aarch64-apple-ios-sim/debug/examples/widget_suite
```

On Intel Macs use `x86_64-apple-ios`. Build outside a Nix shell if its compiler wrappers interfere with the Apple SDK, as described in the iOS platform guide. The runner boots a selected shutdown simulator, waits for boot readiness, executes through `simctl spawn`, and shuts it down afterward. A simulator already booted is left running.

### Android

Install an NDK, an SDK, and the target matching the device's ABI. For an ARM64 device, configure `CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER` to the NDK's `aarch64-linux-android24-clang`, then:

```sh
cargo build -p leptoskit_testing --example widget_suite --target aarch64-linux-android
adb devices
leptoskit test run --platform android --device emulator-5554 \
  --binary target/aarch64-linux-android/debug/examples/widget_suite
```

To start an installed AVD, supply its name and an unused even emulator port:

```sh
leptoskit test run --platform android --device emulator-5556 --avd My_AVD \
  --binary target/aarch64-linux-android/debug/examples/widget_suite
```

`ANDROID_HOME` (or `ANDROID_SDK_ROOT`) locates adb and emulator; otherwise they must be on PATH. The runner waits for Android boot completion, pushes into a unique `/data/local/tmp` directory, quotes remote arguments, executes with an on-device timeout, removes its files, and shuts down only the emulator it started. Existing devices are never reset or shut down. SDK downloads, AVD creation, and physical iOS devices are outside this command.

Arguments after `--` are passed to the test executable, for example `-- --exact counter_updates --nocapture`.

### HarmonyOS

Use DevEco Studio's local emulator or a native HarmonyOS device. Generated apps include a Hypium/UiTest counter journey in `hosts/harmonyos/entry/src/ohosTest`, with bounded waits for the actual ArkUI button and updated text. Run it as a DevEco **Instrument Test**, or build and install its test HAP and invoke `hdc shell aa test`. This checks the real ArkUI, Node-API, and Rust/Wasm path.

The [HarmonyOS testing guide](../platforms/harmonyos.md#testing) covers emulator setup, ABI selection, signing, commands, result checks, artifacts, and SDK-free repository tests. The shared executable runner above does not yet accept HarmonyOS HAPs; device instrumentation remains a separate workflow.

## Reports and CI

Each invocation creates a unique subdirectory under `target/leptoskit-tests` (override with `--artifacts`). It contains stdout/stderr for every command, emulator logs when applicable, and `report.json` with schema version, platform/device, pass/fail, command exit codes, elapsed time, and timeouts. A setup or cleanup failure fails the whole run. Reports are also written for invalid binary paths and failed launches. Tests should write screenshots or other app-specific artifacts explicitly.

Use `--timeout SECONDS` to bound each command; the browser command includes compilation in that deadline. Long-lived services spawned by a test need their own lifecycle management. Retain the artifact directory with CI's upload-artifact step even when tests fail.

| Target        | Widget behavior                              | OS widget rendering                                        | System UI automation                   |
| ------------- | -------------------------------------------- | ---------------------------------------------------------- | -------------------------------------- |
| macOS         | Same Rust tester                             | AppKit tests and PNG snapshots                             | `leptoskit test native` through Appium |
| Windows/Linux | Same Rust tester on CI hosts                 | Win32 and GTK 4 widget harnesses                           | `leptoskit test native` through Appium |
| iOS/Android   | Same Rust tester in simulator/device process | UIKit and Android verifiers on devices                     | `leptoskit test native` through Appium |
| HarmonyOS     | Compiled Wasm guest and render-model tests   | Generated UiTest counter journey; device execution pending | No system-dialog journeys              |
| Web           | Real-browser Leptos component tests          | Browser DOM                                                | Browser component events only          |

A headless test running on Android is evidence for the shared renderer on Android, not proof of Android view behavior. Each widget backend has its own harness that commits real widgets, and `leptoskit test native --suite <suite.json>` runs journeys against a real app, system dialogs included, through an Appium or WebDriver session for iOS, Android, macOS, Windows, and Linux.
