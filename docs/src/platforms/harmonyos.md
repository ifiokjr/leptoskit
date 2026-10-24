# HarmonyOS

<Callout kind="warning" title="Experimental">

HarmonyOS support is early. It renders seven of leptoskit's components, has no code push, and has not yet been run on a device or emulator in CI. Use it to try leptoskit on HarmonyOS, not to ship.

</Callout>

On HarmonyOS, your screens are drawn with ArkUI components inside a Stage-model app, packaged as a HAP for native HarmonyOS devices. Your Rust code runs as WebAssembly, as on iOS and Android, and talks to ArkUI through a Node-API bridge. It does not use an Android APK or ArkWeb.

## What works and what doesn't

| Works                                                                                    | Doesn't work yet                                                                                                                            |
| ---------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------- |
| `View`, `Text`, `Button`, `TextInput`, `PasswordInput`, `ScrollView` (vertical), `Image` | Every other component, including `Switch`, `Checkbox`, `Radio`, `Slider`, the progress indicators, and `Pressable`: they stop with an error |
| Backgrounds, borders, rounded corners, opacity, clipping, sizes, alignment, text colour  | Right-to-left layout, dark mode, and semibold text (drawn as medium)                                                                        |
| Press, text change, submit, focus, and blur events                                       | Scroll events, horizontal scrolling, and native text measurement                                                                            |
| An accessibility label on each element                                                   | Accessibility roles and states                                                                                                              |
| Local `file://` images                                                                   | Images from URLs                                                                                                                            |
| Synchronous Rust plugins listed in `native.plugins`                                      | Code push, Swift and Kotlin plugin handlers, and worker plugins                                                                             |

## Create a project

HarmonyOS has its own starter. It isn't part of the six-platform project that `leptoskit new` creates by default:

```sh
leptoskit new my_app --platform harmonyos
cd my_app
```

The starter is a counter. Its Rust code is in `guest/src/lib.rs`, and the ArkUI app that hosts it is in `hosts/harmonyos`, which you can open and edit in DevEco Studio.

## Build and run

Install [DevEco Studio and the HarmonyOS SDK](https://developer.huawei.com/consumer/en/harmonyos/develop/). The project targets HarmonyOS 5.0.0 (API 12); its `build-profile.json5` is yours and can select another installed SDK. Then:

1. Set `OHOS_NDK_HOME` to the SDK's `native` directory, the one containing `llvm/` and `sysroot/`.
2. Put DevEco's `hvigorw` on your `PATH`, or set `HVIGORW` to its absolute path. Use DevEco's Node.js and `ohpm`.
3. Open `hosts/harmonyos` in DevEco Studio and let it sync, or run `ohpm install` there.
4. Build:

```sh
rustup target add wasm32-unknown-unknown aarch64-unknown-linux-ohos
leptoskit native build harmonyos            # arm64 devices and emulators
leptoskit native build harmonyos --release
```

For an x86-64 emulator, add its target and build that ABI instead:

```sh
rustup target add x86_64-unknown-linux-ohos
leptoskit native build harmonyos-x86_64
```

Plain `leptoskit build` also builds the arm64 app, because the starter sets `native.targets = ["harmonyos"]`. `leptoskit build --platform harmonyos` only works after you add `"harmonyos"` to `app.platforms` in `leptoskit.toml`; the starter doesn't list it, so without that the command stops with "platform `harmonyos` is disabled".

The CLI configures the SDK's Clang, sysroot, and archiver for the selected architecture; Rust's [OpenHarmony platform guide](https://doc.rust-lang.org/stable/rustc/platform-support/openharmony.html) documents the targets. The HAP is written to `hosts/harmonyos/entry/build/default/outputs/default`. To install it, open `hosts/harmonyos` in DevEco Studio, pick an emulator or device, and run. The project has no signing credentials: configure signing and provisioning in DevEco Studio before installing on a device or publishing to AppGallery.

## Testing

Test on a full emulator or a device: that is the only way to exercise your Rust code, the Node-API bridge, and the ArkUI controls together. DevEco's ArkUI Previewer is useful for layout, but it doesn't run leptoskit's runtime.

### Start an emulator and run the app

1. Install DevEco Studio and a compatible HarmonyOS SDK. Open `hosts/harmonyos` and let DevEco sync the project, including `ohpm install`.
2. Open **Tools → Device Manager → Local Emulator**. Create a phone or tablet virtual device, download its image, and start it. The available images and host requirements depend on the installed DevEco version; follow Huawei's [emulator creation guide](https://developer.huawei.com/consumer/cn/doc/doccenter-deveco-studio/ide-emulator-create).
3. Add the SDK's `toolchains` directory to `PATH` and run `hdc list targets`. Select the booted target. Match its ABI: use `leptoskit native build harmonyos` for arm64 or `leptoskit native build harmonyos-x86_64` for x86-64. Do not assume every emulator is x86-64.
4. In DevEco's project signing settings, configure debug signing for the selected target if required. Select the **entry** run configuration and the emulator, then run the app. Check that it displays **Count: 0** and that tapping **Increment** changes the value.

For a physical device, enable developer mode and debugging, connect it, authorize the computer, and select its `hdc` target instead. Use a device running native HarmonyOS with a compatible API level; an Android/EMUI device does not validate this HAP target.

### Run the generated native UI test

New projects include `entry/src/ohosTest/ets/test/Counter.test.ets`, a runner, and an `ohosTest` build target. The test uses [Hypium and UiTest](https://github.com/openharmony/docs/blob/master/en/application-dev/application-test/uitest-guidelines.md) to launch `EntryAbility`, tap the native **Increment** button three times, and verify the displayed count after each tap. It re-finds controls and waits for visible results with a deadline. It does not mock Rust, the guest, the bridge, or ArkUI.

Stop the running app before each test run so its in-memory counter starts at zero. Right-click `Counter.test.ets` in DevEco and run it as an **Instrument Test**, selecting the same target. Inspect the test result panel: `HarmonyCounter.incrementsThroughNativeWidgets` must pass. Change this starter journey when you replace the counter; add journeys for text entry and other controls your app uses.

For a project generated before the test scaffold was added, run `leptoskit native prepare harmonyos` with the updated CLI. This adds missing test sources, but deliberately preserves your root `oh-package.json5`: add `"@ohos/hypium": "1.0.22"` to its `devDependencies`, then run `ohpm install`. This version supports the scaffold's API 12 baseline. The regenerated entry build profile includes the test target. Existing test sources remain application-owned.

### Run instrumentation from a terminal

First build the application for the target ABI with `leptoskit native build` as above. From `hosts/harmonyos`, build the test HAP using the same product and debug signing configuration:

```sh
hvigorw --mode module -p product=default -p module=entry@ohosTest \
  -p buildMode=debug assembleHap --no-daemon
```

Use `$HVIGORW` instead of `hvigorw` if that is how you configured your installation. The test HAP is under `entry/build/default/outputs/ohosTest`; newer DevEco versions can use `entry/.test/default/outputs/ohosTest`. Use the actual paths printed by Hvigor. Install both the app and test HAPs, signed for the selected device where required:

```sh
hdc list targets
export HDC_TARGET='target-id-from-the-list'
export BUNDLE_NAME='com.leptoskit.myapp' # AppScope/app.json5: app.bundleName
export APP_HAP='/absolute/path/to/entry-default-signed.hap'
export TEST_HAP='/absolute/path/to/entry-ohosTest-signed.hap'

hdc -t "$HDC_TARGET" install -r "$APP_HAP"
hdc -t "$HDC_TARGET" install -r "$TEST_HAP"
hdc -t "$HDC_TARGET" shell aa force-stop "$BUNDLE_NAME"
mkdir -p test-artifacts/harmonyos
hdc -t "$HDC_TARGET" shell aa test -b "$BUNDLE_NAME" -m entry_test \
  -s unittest /ets/testrunner/OpenHarmonyTestRunner \
  -s class HarmonyCounter -s timeout 60000 \
  > test-artifacts/harmonyos/instrument.txt 2>&1
```

Check the [instrumentation summary](https://github.com/openharmony/docs/blob/master/en/application-dev/application-test/unittest-guidelines.md#running-test-scripts-in-the-cli), including `OHOS_REPORT_RESULT`: this starter suite must report one test run, one pass, zero failures, and zero errors, with no skipped cases. A successful `hdc` transport exit code alone is not a test verdict. Missing results, a timeout, or zero tests must fail a CI integration. DevEco Instrument Test provides the test verdict and reporting without writing a parser.

### Capture failures and finish device validation

Keep the instrumentation log, SDK/API and ABI information, HAPs, and DevEco test report for each run. With the app visible on the target, [UiTest commands](https://github.com/openharmony/docs/blob/master/en/application-dev/application-test/uitest-guidelines.md#using-commands-to-perform-ui-tests) can capture the rendered result and component tree:

```sh
hdc -t "$HDC_TARGET" shell uitest screenCap -p /data/local/tmp/leptoskit.png
hdc -t "$HDC_TARGET" shell uitest dumpLayout -p /data/local/tmp/leptoskit.json
hdc -t "$HDC_TARGET" file recv /data/local/tmp/leptoskit.png test-artifacts/harmonyos/screen.png
hdc -t "$HDC_TARGET" file recv /data/local/tmp/leptoskit.json test-artifacts/harmonyos/layout.json
```

Also check rotation/resizing, background/foreground transitions, Back followed by relaunch (the counter resets), large text and accessibility labels, and repeated launch/exit cycles. Test hardware APIs and performance on a physical device once your app uses them. Retain screenshots from failed runs instead of treating a HAP build as a UI test.

`leptoskit test run` does not yet orchestrate HarmonyOS HAP installation or instrumentation. Use DevEco or `hdc` for this target. Automated hardware CI needs a provisioned runner with the licensed SDK, a booted emulator or authorized device, and appropriate debug signing. The dedicated hosted CI job above validates SDK builds and guest/render behavior; hardware UI execution is not enabled.

## How it works

Edit `guest/src/lib.rs` for your app and the sources under `hosts/harmonyos` for the ArkUI shell. Running `leptoskit native prepare harmonyos` again keeps your app sources, build configuration, and signing configuration. It regenerates the entry module's `build-profile.json5`, which selects the ABI, the embedded `app.wasm`, and the build inputs in `.leptoskit`.

The bridge owns each runtime through a Node-API object that closes once, explicitly or when it is garbage-collected. The runtime stays on the UI thread, and each render updates the ArkUI components with the frames leptoskit's layout engine computed. Text input, submit, focus, blur, and button events go back to your Rust code. Leaving the page cancels pending work and destroys the runtime.
