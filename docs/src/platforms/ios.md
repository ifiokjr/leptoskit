# iOS

The iOS runtime uses [`leptoskit_wasm`](https://github.com/ifiokjr/leptoskit/tree/main/crates/leptoskit_wasm) and the `wasmi` interpreter. The simulator probe exercises the shared runtime and headless renderer. [Generated native apps](host-projects.md) render the guest's tree with UIKit controls, and can replace that guest through [code push](host-projects.md#code-push).

## Architecture

```
UIKit views
   ▲ mutations (create / update props / insert / layout)
UIKitBackend (leptoskit_uikit, the NativeBackend for UIKit)
   ▲
leptoskit_native renderer (diff, commit, events)     ─┐
leptoskit_yoga layout (flexbox + grid)                ├─ shared, pure Rust
leptoskit_wasm runtime (wasmi, fuel-metered ABI)      │
leptoskit_codepush client (verify + stage updates)   ─┘
   ▲ your app's wasm bundle (code-pushed over the air)
```

This is React Native's shape with the JS engine replaced by a wasm interpreter and the whole stack below the widgets written in Rust. [`leptoskit_uikit`](https://github.com/ifiokjr/leptoskit/tree/main/crates/leptoskit_uikit) is the only UIKit-touching code. Its `UIKitBackend` maps each mutation batch to `UIView` calls, and `UIKitHost` keeps one surface inside a view controller, exactly as React Native's macOS and Windows hosts map the same contract to their toolkits.

## Shipping

The app store artifact is a thin shell: your app icon, permissions, and the leptoskit runtime. Interface changes ship through code push:

```bash
leptoskit native bundle --out dist
leptoskit codepush release --channel production
```

The user downloads an update, then activates it as a trial. The host confirms it once its UIKit views attach, and keeps the working guest if it traps. CI proves the whole journey on a simulator; see [code push in native apps](host-projects.md#code-push). Keep anything that touches new native capabilities (plugins, entitlements, new `Custom` widget kinds) in a store release with a new `native.update_contract`.
