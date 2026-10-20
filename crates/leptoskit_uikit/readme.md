# leptoskit_uikit

Render shared leptoskit primitives as real iOS UIKit views. `UIKitBackend` consumes the existing Yoga mutation stream. It supports containers, labels, images, scrolling, buttons, composed pressables, editable and secure text, switches, checkbox/radio buttons, sliders, progress bars, and loading indicators. The optional `components::helpers` compositions render through those same primitives.

`UIKitHost` retains one `NativeSurface` through a native view controller. The controller keeps the Yoga root inside a stable container, forwards logical bounds and rotation, and reconciles shared events without replacing the host view. Application render functions return ordinary `Element` trees.

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

The app bundle must include `UIApplicationSceneManifest`. The probe's [Info.plist](probe/Info.plist) provides a minimal example. iOS 15 is the supported minimum. Native UIKit values stay on the main thread; text measurement creates independent Core Text values and can run on any thread.

Controls propose typed changes and restore the latest committed value after callbacks, including rejected values and synchronous removal. UIKit has no checkbox/radio class, so those controls use selected `UIButton` instances with SF Symbols and accessibility selection/value traits. A `UISwitch` sits inside an exact Yoga frame and scales its native content to fit. `UISlider` coordinates normalize into the shared `f64` range instead of truncating that range to `f32`. Accessibility increments follow shared step sizes, including adjacent floats.

Images accept local paths, percent-encoded local file URLs, and `resource://name` assets in the application bundle. Network fetching is outside this renderer's image contract. UIKit owns each native editor's interior metrics; use styled parent views when content insets are required.

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
