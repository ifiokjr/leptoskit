# Platforms

leptoskit ships one interface model and runs it everywhere a leptos app can live. This directory is the map: what each platform runs, how you build for it, and how much code you share.

## The one mental model

Write your interface once as leptos components. Each platform provides a **host** that runs those components:

| Platform        | Host                                      | What renders the interface                     |
| --------------- | ----------------------------------------- | ---------------------------------------------- |
| Web (browser)   | leptos CSR + SSR over Axum                | the DOM                                        |
| iOS             | platform shell + `leptoskit_wasm` runtime | native UIKit widgets via `NativeBackend`       |
| Android         | platform shell + `leptoskit_wasm` runtime | native Android views via `NativeBackend`       |
| macOS           | platform shell + `leptoskit_wasm` runtime | native AppKit views via `NativeBackend`        |
| Windows / Linux | platform shell + `leptoskit_wasm` runtime | native WinUI / GTK widgets via `NativeBackend` |

This is the React Native architecture — one component tree, per-platform native hosts — with the difference that the "JS bundle" is a wasm module and the renderer ([`leptoskit_native`](../../crates/leptoskit_native)) is Rust from the diff down to the layout engine ([`leptoskit_yoga`](../../crates/leptoskit_yoga)). React Native macOS, Windows, and Linux proved the model extends past mobile: the host contract is the whole platform-specific surface, and everything above it is shared.

## Where each piece lives

- [`code-reuse.md`](code-reuse.md) — what you write once, what you write per platform, and where the seams are.
- [`web.md`](web.md) — the browser target: SSR, hydration, wasm builds.
- [`ios.md`](ios.md) — the iOS host and simulator validation.
- [`android.md`](android.md) — the Android host and emulator validation.
- [`macos.md`](macos.md) — the macOS desktop host.
- [`windows-linux.md`](windows-linux.md) — the Windows and Linux desktop hosts.

## Verified, not asserted

The [`platform_probe`](../../examples/platform_probe) binary runs the whole stack — layout, renderer diffing, the wasm ABI runtime, and a code push delta update over the real HTTP surface — and its suite runs on the host, the iOS simulator, and an Android emulator:

```bash
devenv shell test:all                      # includes the desktop probe
ios_sim_run                                # see ios.md
android_emulator_run                       # see android.md
```

Every platform this workspace claims to support has the same evidence: the probes either pass on it or the claim is not made.
