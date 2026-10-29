# gallery_verify

Builds the mobile gallery, launches every shared screen on an iOS simulator or Android device, and checks that each one actually rendered.

The [gallery](../readme.md) is seven screens drawn from shared components, and the claim worth testing is that each of them mounts and draws on a phone. The `verify_mobile` example does that end to end. Each screen must report `VERIFIED` from inside the app, which inspects its mounted views and presses the counter's button, and each screenshot goes through `check_rendered`, which decodes the PNG and requires at least 64 distinct colors, so a blank window, a single fill, or a black capture fails.

```sh
cargo run -p gallery_verify --example verify_mobile -- ios --device "$SIMULATOR_UDID"
ANDROID_SERIAL=emulator-5554 ANDROID_ABI=x86_64 cargo run -p gallery_verify --example verify_mobile -- android
```

- `ios` builds `gallery-ios` for the simulator, signs `Gallery.app` ad hoc, installs it, and launches each screen. Pass `--runtime <id> --device-type <id>` instead of `--device` to run on a simulator created and deleted for this run.
- `android` packages an APK (through [`android_apk`](../../../setup/android_apk), without Gradle), installs it on `$ANDROID_SERIAL`, and runs its instrumentation journey.
- Both leave the package and one screenshot per screen in `target/gallery/<platform>` (or `--output <dir>`), and fail unless the device reports `VERIFIED`.

The library holds only the host-side checks: the `SCREENS` list, which the gallery's own tests keep in step with `gallery::SCREENS`, and `check_rendered`. It depends on no widget backend, so the verifier builds on a runner that has no GTK. CI runs the iOS form in the `ios simulator e2e` job and builds and runs the Android form in `android build and widget tests`. The verifier example is not counted toward coverage, because devices exercise it.

Unit tests for the checks run with `cargo test -p gallery_verify`.
