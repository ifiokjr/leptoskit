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

Prefer `Finder::aria_role(Role::Button, "Add")` — like Testing Library's `getByRole`, counting the role a widget kind implies — or accessibility labels where they distinguish controls. `WidgetTester::semantics()` prints the semantics tree a screen reader would expose, for accessibility assertions. `Finder::test_id("counter")` finds an element marked with `.test_id("counter")`; use IDs only when semantics do not identify the widget.

Actions re-query the current tree, reject ambiguous and disabled controls, dispatch through real handlers, and commit one new frame. `enter_text` requires a `TextInput`, and `blur` moves focus out of one through its blur handler; `scroll` requires a `ScrollView` and calls the component's handler. Scrolling does not emulate a native view's internal scroll offset. `find_all` returns owned observations; they cannot be used as stale action targets.

`hold(&finder)` holds a `Button` or `Pressable` down without releasing it, so you can assert its [pressed style](07-shared-components.md#style-the-pressed-state): after `tester.hold(&Finder::text("Add"))?`, `tester.find(&Finder::text("Add"))?.props.background` is its `active:` background. `release()` lets go without pressing; call `tap` for the activation a release usually brings. Neither runs handlers.

`pump_until(&finder, max_pumps)` checks immediately and then advances at most that many render passes. Async test code must advance its runtime before calling `pump`. No arbitrary sleeps or unbounded “settle” loop are involved.

Errors carry the tree, labels, values, frames, and visibility/enabled states. `resize` re-lays out for orientation and viewport tests. Dropping the tester drops the component closure, surface, and handlers.

`WidgetTester::with_backend` accepts a real backend. An AppKit backend must be created and driven on the main thread. Headless metrics approximate fonts; use a platform backend for native text sizing and screenshots.

### Testing Library queries

If you have written tests with Testing Library, you already know how to assert on a screen. `WidgetTester` implements the `Queries` trait, which turns a finder into Testing Library's query families:

```rust
use leptoskit_native::{Element, Role};
use leptoskit_testing::{Queries, WidgetTester};

let tester = WidgetTester::new(|| {
    Element::view()
        .child(Element::text("Inbox"))
        .child(Element::button("Archive"))
}, 390.0, 844.0)?;

tester.get_by_role(Role::Button, "Archive")?;
assert!(tester.query_by_text("Spam")?.is_none());
# Ok::<(), leptoskit_testing::TestError>(())
```

| Query            | No match   | One match     | Several matches |
| ---------------- | ---------- | ------------- | --------------- |
| `get_by_*`       | error      | the match     | error           |
| `get_all_by_*`   | error      | `vec![match]` | every match     |
| `query_by_*`     | `Ok(None)` | `Some(match)` | error           |
| `query_all_by_*` | empty      | `vec![match]` | every match     |

Each family comes `_by_text`, `_by_label`, `_by_role` (an ARIA role and accessible name), and `_by_test_id`, and `get`, `get_all`, `query`, and `query_all` take any `Finder`. The browser tests below implement the same trait, so an assertion reads the same in both.

## Browser tests

A widget test runs your screen against leptoskit's renderer, not a browser. To check what web users get, with real DOM, CSS, and browser events, mount the same component in Chrome or Firefox with `leptoskit_testing_library`. These tests target the web only (`wasm32-unknown-unknown`); for iOS, Android, macOS, Windows, Linux, and HarmonyOS, use `WidgetTester` and the device runners below.

Add `leptoskit_testing_library` and `wasm-bindgen-test` as development dependencies for `wasm32`, with Leptos's `csr` feature enabled in the test build, then write the test:

```rust
use leptos::prelude::{Get, RwSignal, Update};
use leptoskit::prelude::{Button, Element, Text, View, component};
use leptoskit::view;
use leptoskit_testing_library::{Finder, Queries, Role, TestError, render};
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

#[component]
fn Counter(count: RwSignal<u32>) -> Element {
    view! {
        <View>
            <Text>{format!("Count: {}", count.get())}</Text>
            <Button on:press=move || count.update(|count| *count += 1)>"Increment"</Button>
        </View>
    }
}

#[wasm_bindgen_test]
async fn increments() -> Result<(), TestError> {
    let count = RwSignal::new(0);
    let screen = render(move || view! { <Counter count=count /> });

    screen.get_by_text("Count: 0")?;
    screen.tap(&Finder::aria_role(Role::Button, "Increment")).await?;
    screen.get_by_text("Count: 1")?;
    Ok(())
}
```

`render` mounts the component with leptoskit's web renderer, as your web app does, and runs the closure again whenever a signal it reads changes, so create state such as `count` outside it. Dropping the returned `Screen` unmounts the component and removes its markup.

- **Queries.** `Screen` implements `Queries`: `get_by_text`, `query_by_label`, `get_all_by_role`, and the rest behave as in widget tests. Roles come from the HTML the renderer emits, so a `Button` is a `button`, a heading is a `heading`, and a `Switch` is a `switch`. Mark an element with `.test_id("save")` and the browser sees `data-testid="save"`.
- **Waiting.** `find_by_text`, `find_by_label`, `find_by_role`, `find_by_test_id`, and `find_all_by` wait for content that arrives later, such as after a request, checking until it appears or one second passes. `screen.with_timeout(Duration::from_secs(5))` changes the limit.
- **Actions.** `tap`, `enter_text`, and `toggle` find exactly one enabled control, dispatch the click or `input` event a user's action would, and let Leptos re-render before returning. `pump().await` settles Leptos on its own.
- **Failures.** Hidden elements (`hidden`, `aria-hidden`, `display: none`, `visibility: hidden`) never match. Every failed query or action returns a `TestError` that includes the rendered markup.

Run the tests in a browser:

```sh
leptoskit test web --package water_app --browser chrome
leptoskit test web --package water_app --browser firefox
```

Install the `wasm32-unknown-unknown` target, the matching wasm-bindgen test runner, and the requested browser and WebDriver. `CHROMEDRIVER` or `GECKODRIVER` can specify an absolute driver path.

## Run the same widget journey on each OS

A widget test proves your screen behaves; running the same test on each operating system proves it behaves there too. `leptoskit test run` takes a compiled test program, runs it on the host or on a simulator or emulator, and reports the result. Put the journey in a program that exits with an error when it fails, such as an example at `water_app/examples/journey.rs` built from the `WidgetTester` code above.

The runner accepts an already compiled executable. It uses its exit status, including Rust libtest executables, as the verdict. It does not infer success from an empty log or a printed keyword. Application bundles, APKs, and production app UI automation are different artifacts and are not accepted as test executables.

```sh
cargo build -p water_app --example journey
leptoskit test run --binary target/debug/examples/journey
```

On Windows, build on the Windows host and pass `target/debug/examples/journey.exe`. Run the same journey on Windows, Linux, and macOS CI runners to cover all three. The CLI does not install or license a Windows VM on another OS.

### iOS Simulator

Use Xcode on macOS and a simulator runtime already installed on the machine:

```sh
xcrun simctl list devices available
cargo build -p water_app --example journey --target aarch64-apple-ios-sim
leptoskit test run --platform ios --device SIMULATOR-UDID \
  --binary target/aarch64-apple-ios-sim/debug/examples/journey
```

On Intel Macs use `x86_64-apple-ios`. Build outside a Nix shell if its compiler wrappers interfere with the Apple SDK, as described in the iOS platform guide. The runner boots a selected shutdown simulator, waits for boot readiness, executes through `simctl spawn`, and shuts it down afterward. A simulator already booted is left running.

### Android

Install an NDK, an SDK, and the target matching the device's ABI. For an ARM64 device, configure `CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER` to the NDK's `aarch64-linux-android24-clang`, then:

```sh
cargo build -p water_app --example journey --target aarch64-linux-android
adb devices
leptoskit test run --platform android --device emulator-5554 \
  --binary target/aarch64-linux-android/debug/examples/journey
```

To start an installed AVD, supply its name and an unused even emulator port:

```sh
leptoskit test run --platform android --device emulator-5556 --avd My_AVD \
  --binary target/aarch64-linux-android/debug/examples/journey
```

`ANDROID_HOME` (or `ANDROID_SDK_ROOT`) locates adb and emulator; otherwise they must be on PATH. The runner waits for Android boot completion, pushes into a unique `/data/local/tmp` directory, quotes remote arguments, executes with an on-device timeout, removes its files, and shuts down only the emulator it started. Existing devices are never reset or shut down. SDK downloads, AVD creation, and physical iOS devices are outside this command.

Arguments after `--` are passed to the test executable, for example `-- --exact counter_updates --nocapture`.

### HarmonyOS

Use DevEco Studio's local emulator or a native HarmonyOS device. Generated apps include a Hypium/UiTest counter journey in `hosts/harmonyos/entry/src/ohosTest`, with bounded waits for the actual ArkUI button and updated text. Run it as a DevEco **Instrument Test**, or build and install its test HAP and invoke `hdc shell aa test`. This checks the real ArkUI, Node-API, and Rust/Wasm path.

The [HarmonyOS testing guide](../platforms/harmonyos.md#testing) covers emulator setup, ABI selection, signing, commands, result checks, and artifacts. The shared executable runner above does not yet accept HarmonyOS HAPs; device instrumentation remains a separate workflow.

## Reports and CI

Each invocation creates a unique subdirectory under `target/leptoskit-tests` (override with `--artifacts`). It contains stdout/stderr for every command, emulator logs when applicable, and `report.json` with schema version, platform/device, pass/fail, command exit codes, elapsed time, and timeouts. A setup or cleanup failure fails the whole run. Reports are also written for invalid binary paths and failed launches. Tests should write screenshots or other app-specific artifacts explicitly.

Use `--timeout SECONDS` to bound each command; the browser command includes compilation in that deadline. Long-lived services spawned by a test need their own lifecycle management. Retain the artifact directory with CI's upload-artifact step even when tests fail.

Which kind of test covers what, per platform:

| Platform       | Your screen's behavior                                               | Real platform widgets                                      | System UI, such as permission dialogs  |
| -------------- | -------------------------------------------------------------------- | ---------------------------------------------------------- | -------------------------------------- |
| macOS          | `WidgetTester`                                                       | `WidgetTester::with_backend` with the AppKit backend       | `leptoskit test native` through Appium |
| Windows, Linux | `WidgetTester`, run on that OS with `leptoskit test run`             | `WidgetTester::with_backend` with the Win32 or GTK backend | `leptoskit test native` through Appium |
| iOS, Android   | `WidgetTester` on a simulator or emulator, with `leptoskit test run` | The generated app's UI test                                | `leptoskit test native` through Appium |
| HarmonyOS      | Not supported by `leptoskit test run` yet                            | The generated UiTest journey, on an emulator or device     | None                                   |
| Web            | `leptoskit_testing_library` in a real browser                        | The browser's DOM                                          | Browser events only                    |

A `WidgetTester` running on an Android emulator tests leptoskit's renderer on Android, not Android's views. To test what the user actually sees, use the generated app's UI test, or `leptoskit test native --suite <suite.json>`, which drives a real app, system dialogs included, through an Appium or WebDriver session on iOS, Android, macOS, Windows, and Linux.
