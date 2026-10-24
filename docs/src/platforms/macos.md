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

A macOS host uses `NativeBackend` to map mutation batches to AppKit views. The host also owns its window, run loop, environment updates, and application lifecycle. [Code reuse](code-reuse.md) describes how the native and web entrypoints consume the same screen function.

## The AppKit backend

[`leptoskit_appkit`](https://github.com/ifiokjr/leptoskit/tree/main/crates/leptoskit_appkit) is that implementation. `AppKitBackend` maps each element kind to an AppKit widget — flipped, layer-backed `NSView`s for containers, `NSTextField` labels and inputs, `NSButton`, `NSImageView`, and `NSScrollView` — applies backgrounds, rounded corners, borders, and opacity on the views' layers, and measures text with the metrics an `NSTextField` cell uses, so layout reserves exactly what AppKit draws. Button clicks and text edits come back as `NativeEvent`s through one listener.

The [`gallery`](https://github.com/ifiokjr/leptoskit/tree/main/examples/gallery) example has seven screens built with shared component functions, Tailwind classes, and typed styles. AppKit renders the trees offscreen to PNG or in a live window.

```bash
cargo run -p gallery -- snapshot target/gallery   # one 2x PNG per screen
cargo run -p gallery -- window counter            # a live, clickable window
```

Snapshots draw the view tree offscreen, so they need no window and no Screen Recording permission.

## Building for macOS

```bash
cargo build --release --package my_app   # native host binary
leptoskit build --release --platform native
```

Distribution follows the platform: notarized `.app` bundles for the shell, code push channels for the interface. A desktop app and a mobile app can track the same channel — the manifest names the app and the channel, and the same delta machinery serves both.

## Windows and Linux

See [windows-linux.md](windows-linux.md) — the same contract over Win32 and GTK 4, matching how React Native's Windows and macOS ports provide their hosts.
