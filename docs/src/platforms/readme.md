# Platforms

leptoskit shares one component tree between native hosts and the optional Leptos web renderer. This directory describes the available backends, host integration points, and build targets.

## Shared screens and platform backends

Write a screen with `View()`, `Text()`, and the other [shared component builders](../guide/07-shared-components.md). A native surface or `web::render` consumes that same `Element` tree:

| Target            | Implementation                         | Rendering                                                            |
| ----------------- | -------------------------------------- | -------------------------------------------------------------------- |
| Web               | Optional `leptoskit::web`              | Leptos DOM views, server rendering, and hydration                    |
| macOS             | `leptoskit_appkit`                     | Native AppKit views                                                  |
| HarmonyOS         | ArkUI host and the native Wasm runtime | Native widgets and HAP packaging (experimental)                      |
| Headless          | `HeadlessBackend`                      | Recorded mutations and Yoga layout for tests and runtime integration |
| iOS and Android   | Platform host and wasm runtime probes  | Native widget backends remain platform integration work              |
| Windows and Linux | `NativeBackend` integration contract   | Native widget backends are not included                              |

The native renderer ([`leptoskit_native`](https://github.com/ifiokjr/leptoskit/tree/main/crates/leptoskit_native)) diffs component trees and delegates widget operations to a backend. [`leptoskit_yoga`](https://github.com/ifiokjr/leptoskit/tree/main/crates/leptoskit_yoga) computes native layout. The browser uses the same utility declarations as CSS and delegates layout to its CSS engine. The [coverage reference](../reference/css-coverage.md) distinguishes shared support from unsupported CSS behavior.

## Where each piece lives

- [`code-reuse.md`](code-reuse.md) — what you write once, what you write per platform, and where the seams are.
- [`web.md`](web.md) — the browser target: SSR, hydration, wasm builds.
- [`ios.md`](ios.md) — the iOS host and simulator validation.
- [`android.md`](android.md) — the Android host and emulator validation.
- [`harmonyos.md`](harmonyos.md) — the experimental ArkUI host and HAP packaging.
- [`macos.md`](macos.md) — the macOS desktop host.
- [`windows-linux.md`](windows-linux.md) — the Windows and Linux desktop hosts.

## Runtime probes

The [`platform_probe`](https://github.com/ifiokjr/leptoskit/tree/main/examples/platform_probe) binary exercises layout, renderer diffing, the wasm ABI runtime, and a code push delta update over HTTP. Its suite runs on the host, the iOS simulator, and an Android emulator:

```bash
devenv shell test:all                      # includes the desktop probe
ios_sim_run                                # see ios.md
android_emulator_run                       # see android.md
```

The probes verify these runtime contracts. They do not demonstrate a UIKit or Android widget backend, or complete CSS support.
