# HarmonyOS: repository checks and CI

The public [HarmonyOS page](../src/platforms/harmonyos.md) explains how to build and test an app. This file covers how the repository validates the experimental target without a device.

## Run the checks without a device

From the Leptoskit repository:

```sh
devenv shell test:harmonyos
devenv shell cargo test --locked -p leptoskit_cli harmonyos
devenv shell cargo test --locked -p leptoskit_cli --test wasm_pipeline harmonyos_counter -- --ignored
```

The last command compiles a fresh Wasm guest and needs the `wasm32-unknown-unknown` Rust target. CI runs these checks. They do not exercise HarmonyOS widgets or the platform's native library loader.

## Dedicated HarmonyOS CI

The `harmonyos` workflow runs on pull requests, pushes to `main`, and manual dispatch. Its **harmonyos SDK, guest, and render tests** job uses the public OpenHarmony 5.0.0.71 API 12 SDK and pinned Hvigor 5.2.2 tools. It runs the host contracts, Deno render tests, and compiled Wasm counter, then builds a fresh app for arm64 debug and x86-64 release and compiles each ABI's Hypium instrumentation HAP. It checks the app archive for the native library, compiled ArkUI, and embedded Wasm, and retains the four HAPs and command logs as workflow artifacts even if a later build fails.

The SDK and Hypium downloads have pinned checksums; `setup/harmonyos/package-lock.json` pins the build-tool dependency tree. Compatibility edits to OpenHarmony product settings affect only the generated CI fixture. A failed SDK build fails the job; it is not treated as an optional check. No Huawei account or commercial SDK is required for this job.

To run the same build test locally, install `setup/harmonyos` with `npx --yes npm@10.9.4 ci --prefix setup/harmonyos --ignore-scripts`. npm 10 is pinned because npm 11 rejects the older vendor bundle's optional dependency metadata. Set `OHOS_SDK_HOME` to a public SDK root containing `12/native`, `12/ets`, `12/js`, and `12/toolchains`, and `HVIGORW` to the absolute path of `setup/harmonyos/hvigorw`. Extract the official Hypium 1.0.22 HAR into a directory and set `LEPTOSKIT_HYPIUM` to that directory containing `oh-package.json5`. Install both OHOS Rust targets, then run:

```sh
devenv shell test:harmonyos:sdk
```

This job compiles the instrumentation suite but does not execute its UI assertions. Emulator/device execution still requires a provisioned HarmonyOS target and appropriate SDK access and signing. There is no green emulator verdict implied by a successful SDK job.

## Evidence so far

The scaffold's guest is compiled and exercised in the existing coverage job, including repeated presses, viewport changes, and fresh-runtime state. Host tests cover generation, ABI selection, asset copying, capability failures, and packaging-tool failures. The render transaction model is checked with Deno. The generated instrumentation suite is a separate device validation step.

Local compatibility builds produced unsigned arm64 debug and x86-64 release HAPs with the public OpenHarmony 5.0.0.71 (API 12) SDK and Hvigor 5.2.2. These builds compile ArkUI, link the Rust/Node-API runtime, and package the guest. The generated instrumentation HAP also compiled and packaged with Hypium 1.0.22; its UI assertions have not been executed here. The compatibility check changes the product to `runtimeOS: "OpenHarmony"`, uses numeric SDK version `12`, and replaces the `phone` device type with `default` for that public SDK. The generated application retains Huawei's HarmonyOS configuration. Huawei SDK validation, signing, installation, and interaction tests on a device or emulator are still required before production use.
