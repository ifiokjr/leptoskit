# android_apk

Builds, signs, installs, and inspects the debug APKs of the repository's Android device verifiers, without Gradle.

Two verifiers need a real Android app to run on a device: `verify_device` in `leptoskit_android` (the widget probe) and `verify_mobile` in `gallery_verify` (the gallery journey). Each is a Rust library plus a small Kotlin Activity. Packaging that with Gradle would add a build system and a project template to maintain, so this crate does it directly with the Android SDK tools:

1. `Toolchain::rustc` cross-compiles a crate as a shared library with the NDK linker.
2. `Toolchain::package` compiles the Kotlin host against a pinned Kotlin stdlib (downloaded once, checked against `KOTLIN_STDLIB_SHA256` on every build), dexes it with D8, links the manifest with aapt2, adds the library and dex files, aligns the archive, and signs it with a local debug keystore.
3. `Device::install` replaces any earlier install, and `Device::read_app_file` pulls the evidence the app writes into its private storage.

It runs on the host and depends on no widget backend. `Settings::from_env` reads `ANDROID_HOME` (or `ANDROID_SDK_ROOT`, required), `ANDROID_NDK_HOME` or `ANDROID_NDK_VERSION`, `ANDROID_ABI` (`arm64-v8a` by default, or `x86_64` for emulators), `ANDROID_API_LEVEL`, `ANDROID_BUILD_TOOLS_VERSION`, `JAVA_HOME`, `CARGO_TARGET_DIR`, and `ANDROID_SERIAL`. `kotlinc`, `java`, `curl`, and `zip` must be on `PATH`.

Nobody runs this crate directly. It is a dependency of the two verifiers, which the `android build and widget tests` CI job runs on an API 35 emulator:

```sh
ANDROID_SERIAL=emulator-5554 ANDROID_ABI=x86_64 cargo run -p gallery_verify --example verify_mobile -- android
```

Its tests (`cargo test -p android_apk`) cover the packaging steps against fake tools, so they run without an SDK.
