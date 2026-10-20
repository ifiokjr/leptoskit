# iOS

The iOS runtime uses [`leptoskit_wasm`](https://github.com/ifiokjr/leptoskit/tree/main/crates/leptoskit_wasm) and the `wasmi` interpreter. The simulator probe exercises the shared runtime and headless renderer. [Generated native apps](host-projects.md) render the guest's tree with UIKit controls, and can replace that guest through [code push](host-projects.md#code-push).

## Architecture

```
UIKit views
   ▲ mutations (create / update props / insert / layout)
NativeBackend (the platform half you implement against UIKit)
   ▲
leptoskit_native renderer (diff, commit, events)     ─┐
leptoskit_yoga layout (flexbox + grid)                ├─ shared, pure Rust
leptoskit_wasm runtime (wasmi, fuel-metered ABI)      │
leptoskit_codepush client (verify + stage updates)   ─┘
   ▲ your app's wasm bundle (code-pushed over the air)
```

This is React Native's shape with the JS engine replaced by a wasm interpreter and the whole stack below the widgets written in Rust. The `NativeBackend` implementation is the only UIKit-touching code: it maps each mutation batch to `UIView` calls, exactly as React Native's macOS and Windows hosts map the same contract to AppKit and WinUI.

## Building for the simulator

```bash
rustup target add aarch64-apple-ios-sim
cargo build --release --package my_app --target aarch64-apple-ios-sim
```

Link with Xcode's clang (the `xcrun --find clang` toolchain), not a nix-wrapped compiler — see [`setup/scripts/ios_probe_run.sh`](https://github.com/ifiokjr/leptoskit/blob/main/setup/scripts/ios_probe_run.sh) or the CI workflow for the exact environment.

## Validating on the simulator

The [`platform_probe`](https://github.com/ifiokjr/leptoskit/tree/main/examples/platform_probe) suite is the continuous evidence that the stack runs on iOS, not just for iOS: the same binary that runs as a desktop test also installs and runs on the simulator.

```bash
# Wrap the probe as a minimal app bundle (script in the probe's directory)
cargo build --release --package platform_probe --target aarch64-apple-ios-sim
xcrun simctl boot "iPhone 17 Pro"   # or any available device
xcrun simctl install booted target/platform_probe.app
xcrun simctl launch --console-pty booted dev.leptoskit.platform-probe
```

Expected output, on device:

```
PASS yoga:flexbox
PASS yoga:grid
PASS native:diff-and-events
PASS wasm:abi-runtime
PASS codepush:delta-update
platform probe: all probes passed on ios
```

The `codepush:delta-update` probe ingests two releases into a real store, serves them through the real axum router, and updates a client over the wire with a binary delta that hash-verifies before staging — all on the simulator OS.

## Shipping

The app store artifact is a thin shell: your app icon, permissions, and the leptoskit runtime. Interface changes ship through code push:

```bash
leptoskit native bundle --out dist
leptoskit codepush release --channel production
```

The user downloads an update, then activates it as a trial. The host confirms it once its UIKit views attach, and keeps the working guest if it traps or renders nothing. CI proves the whole journey on a simulator; see [code push in native apps](host-projects.md#code-push). Keep anything that touches new native capabilities (plugins, entitlements, new `Custom` widget kinds) in a store release with a new `native.update_contract`.
