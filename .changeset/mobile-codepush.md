---
"release": minor
---

# Code push the Wasm guest of generated iOS and Android apps

Generated native apps can now download and activate a replacement Wasm guest. With `codepush.enabled`, the host shows an update bar. **Check for update** downloads a release on a background worker and verifies its signature, target, and version. It also checks the release's `runtime.json` against the installed shell's ABI, `native.update_contract`, and plugins. **Activate update** runs the release as a trial beside the working guest. It is confirmed only after its native root attaches. A release that traps or renders nothing is rejected, and the working guest keeps running. A trial interrupted by process death is rejected on the next launch, and confirmed releases load offline with their signatures checked again.

`leptoskit native bundle` builds the guest and its `runtime.json` for `leptoskit codepush release`, and `native prepare` embeds the guest as the app's trusted store baseline, together with its update configuration. `leptoskit_bridge_ffi` exposes `InstalledBundles` to Swift and Kotlin as `lk_updates_*`, and each host drives it from one background worker. Release builds require HTTPS; `codepush.allow_http` permits a plaintext registry in debug builds only.

A `mobile_codepush` example runs the whole journey on a simulator or emulator, and CI runs it on both.
