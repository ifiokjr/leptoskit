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

## Ownership and runtime

Edit `guest/src/lib.rs` for application behavior and the sources under `hosts/harmonyos` for the ArkUI shell. Repeated prepares preserve existing app sources, root build configuration, and signing configuration. The entry `build-profile.json5` selects the generated ABI; that file, the embedded `app.wasm`, and `.leptoskit` build inputs are regenerated.

The bridge owns each runtime through a Node-API object with explicit, idempotent close and garbage-collection cleanup. Runtime operations remain on the UI thread; render transactions update native widgets with Yoga's logical viewport frames. Text input, submit, focus, blur, and button events return through the existing guest ABI. Pending synchronous plugin work yields to the next UI turn. Leaving the page cancels its pending pump and destroys the runtime.

## Current limits

This initial host implements containers, text, buttons, controlled text/password inputs, vertical scroll containers, and file images. Unsupported widget kinds fail visibly. Rich text runs, the remaining controls, native text measurement, dark-mode propagation, automated device selection/deployment, and code push need further work. Code push and selected platform/worker plugins fail preparation with a diagnostic. Synchronous Rust plugins can be selected through `native.plugins`; Android and Apple handlers do not run on HarmonyOS.

The scaffold's guest is compiled and exercised in the existing coverage job, including repeated presses, viewport changes, and fresh-runtime state. Host tests cover generation, ABI selection, asset copying, capability failures, and packaging-tool failures. The render transaction model is checked with Deno.

Local compatibility builds produced unsigned arm64 debug and x86-64 release HAPs with the public OpenHarmony 5.0.0.71 (API 12) SDK and Hvigor 5.2.2. These builds compile ArkUI, link the Rust/Node-API runtime, and package the guest. The compatibility check changes the product to `runtimeOS: "OpenHarmony"`, uses numeric SDK version `12`, and replaces the `phone` device type with `default` for that public SDK. The generated application retains Huawei's HarmonyOS configuration. Huawei SDK validation, signing, installation, and interaction tests on a device or emulator are still required before production use.
