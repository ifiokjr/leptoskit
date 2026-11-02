<h1 align="center">leptoskit_uikit</h1>

<p align="center">
  <strong>leptoskit_uikit</strong> is the iOS backend: it renders shared leptoskit primitives as real `UIView` and `UIViewController` content, with UIKit text measurement and native controls.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://docs.rs/leptoskit_uikit"><strong>Docs</strong></a> ·
  <a href="https://github.com/ifiokjr/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<p align="center">
  <a href="https://crates.io/crates/leptoskit_uikit"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_uikit.svg"></a>
  <a href="https://docs.rs/leptoskit_uikit"><img alt="docs.rs" src="https://docs.rs/leptoskit_uikit/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<br>

## Overview

Render shared leptoskit primitives as real iOS UIKit views. `UIKitBackend` consumes the existing Yoga mutation stream. It supports containers, labels, images, scrolling, buttons, composed pressables, editable and secure text, switches, checkbox/radio buttons, sliders, progress bars, loading indicators, and vector drawings. The optional `components::helpers` compositions render through those same primitives.

`UIKitHost` retains one `NativeSurface` through a native view controller. The controller keeps the Yoga root inside a stable container, forwards logical bounds and rotation, and reconciles shared events without replacing the host view. Application render functions return ordinary `Element` trees.

The app bundle must include `UIApplicationSceneManifest`. The probe's [Info.plist](probe/Info.plist) provides a minimal example. iOS 15 is the supported minimum. Native UIKit values stay on the main thread; text measurement creates independent Core Text values and can run on any thread.

Controls propose typed changes and restore the latest committed value after callbacks, including rejected values and synchronous removal. UIKit has no checkbox/radio class, so those controls use selected `UIButton` instances with SF Symbols and accessibility selection/value traits. A `UISwitch` sits inside an exact Yoga frame and scales its native content to fit. `UISlider` coordinates normalize into the shared `f64` range instead of truncating that range to `f32`. Accessibility increments follow shared step sizes, including adjacent floats.

Images accept local paths, percent-encoded local file URLs, and `resource://name` assets in the application bundle. Network fetching is outside this renderer's image contract. UIKit owns each native editor's interior metrics; use styled parent views when content insets are required.

Vector drawings paint each path with its own `CAShapeLayer`, in order, so a path's fill sits under its stroke and later paths cover earlier ones. Points map from the view box onto the frame before the `CGPath` is built, so stroke widths stay in points under any scaling. Layout and drawing changes redraw at once, without implicit layer animations. Shapes paint outside the frame unless the element clips. A labelled drawing is one image accessibility element; an unlabelled one is decorative.

Determinate bars use `UIProgressView`. Indeterminate bars use a clipped native Core Animation segment. Spinners use `UIActivityIndicatorView`. Hidden, stopped, deleted, and dropped controls stop animations; a stopped spinner hides while its Yoga layout space remains.

Run the real simulator verifier on macOS with Xcode and an installed runtime:

```sh
CARGO_BUILD_JOBS=2 CARGO_TARGET_DIR=/tmp/leptoskit-uikit-target \
  cargo run -p leptoskit_uikit --example verify_simulator -- \
  --runtime com.apple.CoreSimulator.SimRuntime.iOS-26-5 \
  --device-type com.apple.CoreSimulator.SimDeviceType.iPhone-17 \
  --output /tmp/leptoskit-uikit-verification
```

It builds and signs the probe, creates its own simulator, installs an actual UIApplication, exercises native classes/actions/editors/layout/lifecycle, writes `native-report.tsv` and `native-widgets.png`, and removes that simulator. It never selects an existing device. A cross-build alone is not a renderer verification result.

## How it fits

It consumes the same Yoga mutation stream as every other backend. The generated iOS app hosts it through `UIKitHost`; the backend crate draws a wider component set than the Swift renderer used by the generated app.

## Use it

```rust,ignore
use leptoskit_native::Element;
use leptoskit_uikit::{MainThreadMarker, UIKitHost, run_application};

let mtm = MainThreadMarker::new().expect("UIKit main thread");
run_application(mtm, |mtm, scene| {
    let host = UIKitHost::new(mtm, || {
        Element::view().class("w-full h-full p-6 bg-white").child(
            Element::text("Shared primitives, native pixels"),
        )
    });
    host.set_dark_mode(false);
    host.open_window(scene)
});
```

## Learn more

- [Crate documentation](https://docs.rs/leptoskit_uikit)
- [leptoskit documentation](https://ifiokjr.github.io/leptoskit/)
- [Repository](https://github.com/ifiokjr/leptoskit)
- [iOS](https://ifiokjr.github.io/leptoskit/platforms/ios.html)
