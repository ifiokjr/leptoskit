# HarmonyOS

The experimental HarmonyOS target embeds the native Rust/Wasm runtime in a Stage-model application. ArkUI renders native widgets through a Node-API bridge. The target packages a HAP for native HarmonyOS devices; it does not use the Android APK route or ArkWeb.

## Create an application

```sh
leptoskit new my_app --platform native --native-target harmonyos
cd my_app
```

The scaffold contains a Rust counter guest with no platform plugin dependencies, its installed Rust library, and editable ArkUI application sources under `hosts/harmonyos`. Set `--sdk-path /path/to/leptoskit` when developing against an unpublished checkout.

## Build and run

Install [DevEco Studio and the HarmonyOS SDK](https://developer.huawei.com/consumer/en/harmonyos/develop/). The project starts with HarmonyOS 5.0.0 (API 12); its root `build-profile.json5` is app-owned and can select your installed SDK. Set `OHOS_NDK_HOME` to the SDK's `native` directory containing `llvm/` and `sysroot/`. Use DevEco's Node.js and `ohpm`. Put DevEco's `hvigorw` on PATH or set `HVIGORW` to its absolute path, then sync `hosts/harmonyos` in DevEco Studio or run `ohpm install` there.

```sh
rustup target add wasm32-unknown-unknown aarch64-unknown-linux-ohos
leptoskit native prepare harmonyos
leptoskit native build harmonyos
leptoskit native build harmonyos --release
```

An x86-64 emulator needs the matching Rust target and native ABI:

```sh
rustup target add x86_64-unknown-linux-ohos
leptoskit native build harmonyos-x86_64
```

The CLI configures the SDK's Clang, sysroot, and archiver for the selected architecture. Rust documents these targets in its [OpenHarmony platform support guide](https://doc.rust-lang.org/stable/rustc/platform-support/openharmony.html). `leptoskit build` selects the arm64 target when `native.targets = ["harmonyos"]`.

HAP output is under `hosts/harmonyos/entry/build/default/outputs/default`. Open `hosts/harmonyos` in DevEco Studio to select an emulator or device and run the app. Configure signing and provisioning in DevEco Studio for device installation and AppGallery distribution. The generated project has no signing credentials.

## Testing

HarmonyOS has a [local emulator in DevEco Studio](https://developer.huawei.com/consumer/en/deveco-studio/). Use a full emulator or physical device to test the Rust library, Node-API bridge, and native ArkUI controls together. ArkUI Previewer is useful for layout work but does not establish that this native runtime works.

| Check                     | What it exercises                                                                            | Requirements                                                             |
| ------------------------- | -------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------ |
| Rust host and guest tests | Scaffolding, tool failures, the compiled counter's events, resizing, and fresh runtime state | Repository development environment; no HarmonyOS SDK                     |
| Render-model tests        | Render mutations, controlled text, colors, and unsupported widgets                           | Deno; no HarmonyOS SDK                                                   |
| HAP builds                | Rust cross compilation, Node-API linking, ArkUI compilation, and packaging for each ABI      | Compatible SDK, Hvigor, Node.js, and ohpm                                |
| Instrument Test           | A native button tap, the real Wasm guest, and the updated native text                        | Running emulator/device, matching HAPs, and debug signing where required |

### Run the checks without a device

From the Leptoskit repository:

```sh
devenv shell test:harmonyos
devenv shell cargo test --locked -p leptoskit_cli harmonyos
devenv shell cargo test --locked -p leptoskit_cli --test wasm_pipeline harmonyos_counter -- --ignored
```

The last command compiles a fresh Wasm guest and needs the `wasm32-unknown-unknown` Rust target. CI runs these checks. They do not exercise HarmonyOS widgets or the platform's native library loader.

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

`leptoskit test run` does not yet orchestrate HarmonyOS HAP installation or instrumentation. Use DevEco or `hdc` for this target. Automated hardware CI needs a provisioned runner with the licensed SDK, a booted emulator or authorized device, and appropriate debug signing; it is not enabled in this repository's hosted CI.

## Ownership and runtime

Edit `guest/src/lib.rs` for application behavior and the sources under `hosts/harmonyos` for the ArkUI shell. Repeated prepares preserve existing app sources, root build configuration, and signing configuration. The entry `build-profile.json5` selects the generated ABI; that file, the embedded `app.wasm`, and `.leptoskit` build inputs are regenerated.

The bridge owns each runtime through a Node-API object with explicit, idempotent close and garbage-collection cleanup. Runtime operations remain on the UI thread; render transactions update native widgets with Yoga's logical viewport frames. Text input, submit, focus, blur, and button events return through the existing guest ABI. Pending synchronous plugin work yields to the next UI turn. Leaving the page cancels its pending pump and destroys the runtime.

## Current limits

This initial host implements containers, text, buttons, controlled text/password inputs, vertical scroll containers, and file images. Unsupported widget kinds fail visibly. Rich text runs, the remaining controls, native text measurement, dark-mode propagation, automated device selection/deployment, and code push need further work. Code push and selected platform/worker plugins fail preparation with a diagnostic. Synchronous Rust plugins can be selected through `native.plugins`; Android and Apple handlers do not run on HarmonyOS.

The scaffold's guest is compiled and exercised in the existing coverage job, including repeated presses, viewport changes, and fresh-runtime state. Host tests cover generation, ABI selection, asset copying, capability failures, and packaging-tool failures. The render transaction model is checked with Deno. The generated instrumentation suite is a separate device validation step.

Local compatibility builds produced unsigned arm64 debug and x86-64 release HAPs with the public OpenHarmony 5.0.0.71 (API 12) SDK and Hvigor 5.2.2. These builds compile ArkUI, link the Rust/Node-API runtime, and package the guest. The generated instrumentation HAP also compiled and packaged with Hypium 1.0.22; its UI assertions have not been executed here. The compatibility check changes the product to `runtimeOS: "OpenHarmony"`, uses numeric SDK version `12`, and replaces the `phone` device type with `default` for that public SDK. The generated application retains Huawei's HarmonyOS configuration. Huawei SDK validation, signing, installation, and interaction tests on a device or emulator are still required before production use.
