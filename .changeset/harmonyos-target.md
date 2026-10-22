---
"release": minor
---

# Add an experimental native HarmonyOS deployment target

Generate an ArkUI counter with `leptoskit new app --platform native --native-target harmonyos`. Prepare or package arm64 and x86-64 HarmonyOS hosts with `native prepare/build harmonyos` or `harmonyos-x86_64`, using the DevEco native toolchain and Hvigor to produce a HAP. The host embeds the existing Wasm interpreter and renders native ArkUI widgets. It rejects unsupported platform/worker plugins and code push. Both architectures have been packaged with the public OpenHarmony SDK; Huawei SDK and device validation remain required before production use.

Generated apps include a Hypium/UiTest counter journey and instructions for DevEco emulator/device instrumentation, SDK-free checks, and failure artifacts.

A dedicated HarmonyOS CI job runs guest and render tests and builds real app and instrumentation HAPs for arm64 debug and x86-64 release with the pinned public SDK. Device UI execution remains a separate validation step.
