# Platforms

leptoskit shares one component tree between native hosts and the optional Leptos web renderer. This directory describes the available backends, host integration points, and build targets.

## Shared screens and platform backends

Write a screen with `View()`, `Text()`, and the other [shared component builders](../guide/07-shared-components.md). A native surface or `web::render` consumes that same `Element` tree:

| Target    | Implementation                         | Rendering                                                             |
| --------- | -------------------------------------- | --------------------------------------------------------------------- |
| Web       | Optional `leptoskit::web`              | Leptos DOM views, server rendering, and hydration                     |
| macOS     | `leptoskit_appkit` (`AppKitBackend`)   | AppKit views. The most complete backend; it renders every screenshot. |
| iOS       | `leptoskit_uikit` (`UIKitBackend`)     | UIKit views, verified on the simulator in CI                          |
| Android   | `leptoskit_android` (`AndroidBackend`) | Android views through a Kotlin `WidgetHost`, verified on an emulator  |
| Linux     | `leptoskit_gtk` (`GtkBackend`)         | GTK 4 widgets, verified under Xvfb in CI                              |
| Windows   | `leptoskit_windows` (`WindowsBackend`) | Win32 windows and native common controls, verified on Windows in CI   |
| HarmonyOS | ArkUI host and the native Wasm runtime | Native widgets and HAP packaging (experimental)                       |
| Headless  | `HeadlessBackend`                      | Recorded mutations and Yoga layout for tests and runtime integration  |

Each backend is a feature of the `leptoskit` crate: `appkit`, `uikit`, `android`, `gtk`, and `windows`. AppKit is the most mature. The others cover every built-in primitive, but they are younger, so expect gaps in controls and styling.

The native renderer ([`leptoskit_native`](https://github.com/ifiokjr/leptoskit/tree/main/crates/leptoskit_native)) diffs component trees and delegates widget operations to a backend. [`leptoskit_yoga`](https://github.com/ifiokjr/leptoskit/tree/main/crates/leptoskit_yoga) computes native layout. The browser uses the same utility declarations as CSS and delegates layout to its CSS engine. The [coverage reference](../reference/css-coverage.md) distinguishes shared support from unsupported CSS behavior.

## Where each piece lives

- [`code-reuse.md`](code-reuse.md) — what you write once, what you write per platform, and where the seams are.
- [`web.md`](web.md) — the browser target: SSR, hydration, wasm builds.
- [`ios.md`](ios.md) — the iOS host and simulator validation.
- [`android.md`](android.md) — the Android host and emulator validation.
- [`harmonyos.md`](harmonyos.md) — the experimental ArkUI host and HAP packaging.
- [`macos.md`](macos.md) — the macOS desktop host.
- [`windows-linux.md`](windows-linux.md) — the Windows and Linux desktop hosts.
