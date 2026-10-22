---
"release": minor
---

# Add an experimental native HarmonyOS deployment target

Generate an ArkUI counter with `leptoskit new app --platform native --native-target harmonyos`. Prepare or package arm64 and x86-64 HarmonyOS hosts with `native prepare/build harmonyos` or `harmonyos-x86_64`, using the DevEco native toolchain and Hvigor to produce a HAP. The host embeds the existing Wasm interpreter and renders native ArkUI widgets. It rejects unsupported platform/worker plugins and code push. Both architectures have been packaged with the public OpenHarmony SDK; Huawei SDK and device validation remain required before production use.
