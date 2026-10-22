# HarmonyOS native counter

The Rust Wasm guest in `guest/src/lib.rs` runs in the installed native runtime. ArkUI renders its widgets; this application does not use ArkWeb.

Install DevEco Studio with the HarmonyOS 5.0.0 (API 12) SDK and its native toolchain. Set `OHOS_NDK_HOME` to the SDK's `native` directory (the one containing `llvm` and `sysroot`). Put DevEco's `hvigorw` on PATH, or set `HVIGORW` to its absolute path. Use DevEco's Node.js and `ohpm`, and sync `hosts/harmonyos` in the IDE or run `ohpm install` there.

```sh
rustup target add wasm32-unknown-unknown aarch64-unknown-linux-ohos
leptoskit native prepare harmonyos
leptoskit native build harmonyos
# x86-64 emulator:
rustup target add x86_64-unknown-linux-ohos
leptoskit native build harmonyos-x86_64
```

`leptoskit build` builds the arm64 HarmonyOS target. HAPs appear in `hosts/harmonyos/entry/build/default/outputs/default`. Open `hosts/harmonyos` in DevEco Studio to select a device/emulator and run. Configure the application's signing in DevEco before installing on a physical device or distributing through AppGallery; the scaffold contains no signing keys.

Edit ArkUI sources and application settings under `hosts/harmonyos`. The entry `build-profile.json5`, embedded `app.wasm`, and `.leptoskit` build inputs are regenerated. Keep your other changes in version control. Running `prepare` preserves the application sources.

This initial host supports views, text, buttons, controlled text/password inputs, vertical scroll containers, and file images. Other widget kinds fail visibly. Native platform and worker plugins, code push, rich inline text, native text measurement, dark-mode propagation, and automated device deployment are not implemented. Synchronous Rust plugins can be selected in `native.plugins`. Restarting the app creates a fresh guest; navigation away releases it.

The host and build pipeline require validation with a HarmonyOS SDK and device before production use.
