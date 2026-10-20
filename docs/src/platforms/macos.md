# macOS (desktop, native components)

Desktop is where the "no web view" claim pays off most visibly: a leptoskit macOS app draws with AppKit views, lays them out with the same Rust engine as the phone builds, and updates over the air like the mobile apps.

## How React Native's desktop hosts frame it — and ours

React Native macOS, Windows, and Linux established the pattern a desktop adoption needs: **the host contract is the only platform-specific code**. RN macOS implements React's view managers over AppKit; RN Windows over XAML/Composition; RN Linux over GTK. Everything above the contract — the component model, reconciliation, layout, the packager — is shared.

leptoskit takes the same split, in Rust:

| React Native desktop       | leptoskit equivalent                                                                                   |
| -------------------------- | ------------------------------------------------------------------------------------------------------ |
| view managers per platform | [`NativeBackend`](https://github.com/ifiokjr/leptoskit/tree/main/crates/leptoskit_native) per platform |
| shadow tree / Fabric diff  | `leptoskit_native` mutation batches                                                                    |
| yoga (C)                   | `leptoskit_yoga` (pure Rust, flexbox + grid)                                                           |
| JS bundle + JS engine      | wasm bundle + `wasmi` (pure Rust)                                                                      |
| Metro / code push          | `leptoskit` CLI + `leptoskit_codepush`                                                                 |

A macOS host is one `NativeBackend` implementation mapping mutation batches to `NSView` calls (`NSView`, `NSTextField`, `NSImageView`, `NSScrollView`…), plus the three shell responsibilities (boot the runtime, pump the run loop, check for updates) documented in [code-reuse.md](code-reuse.md).

## Validating the desktop stack today

The desktop run of the [`platform_probe`](https://github.com/ifiokjr/leptoskit/tree/main/examples/platform_probe) suite is part of `devenv shell test:all` — the same five probes, including the code push delta update over the real HTTP surface, execute as a native macOS process:

```bash
devenv shell test:all
```

Because the probe runs the renderer, the layout engine, the wasm runtime, and the update client as a native arm64 binary on macOS, it is the desktop-native evidence: every layer below the widgets runs on the desktop OS today, and the widget layer is the `NativeBackend` implementation above.

## Building for macOS

```bash
cargo build --release --package my_app   # native host binary
leptoskit build --release --platform native
```

Distribution follows the platform: notarized `.app` bundles for the shell, code push channels for the interface. A desktop app and a mobile app can track the same channel — the manifest names the app and the channel, and the same delta machinery serves both.

## Windows and Linux

See [windows-linux.md](windows-linux.md) — the same contract over WinUI and GTK, matching how RN Windows and RN Linux provide their hosts.
