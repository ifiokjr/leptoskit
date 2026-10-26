# Android

On Android, your screens are drawn with Android views: a `Button` is an `android.widget.Button`, a `TextInput` an `EditText`, a `ScrollView` a `ScrollView`. The app that `leptoskit new` generates is a small Kotlin activity that runs your screens as WebAssembly, so it can also replace them over the air with [code push](../guide/05-code-push.md).

## What you need

- Java 17 or newer, the Android SDK with platform 35, CMake, and an NDK. Set `ANDROID_HOME` to the SDK; `ANDROID_NDK_HOME` picks a specific NDK.
- The Rust targets for your device or emulator, and for WebAssembly:

```bash
rustup target add aarch64-linux-android x86_64-linux-android wasm32-unknown-unknown
```

## Build and run

```bash
leptoskit build --platform android
```

This builds an arm64 debug APK, for phones and for emulators on Apple silicon, at `android/app/build/outputs/apk/debug/app-debug.apk`. Install it on a running emulator or a connected phone:

```bash
adb install -r android/app/build/outputs/apk/debug/app-debug.apk
```

For an x86-64 emulator, build `leptoskit native build android-x86_64` instead. Add `--release` for a release APK; configure release signing in `android/app/build.gradle.kts` before you distribute it.

You can also open the `android/` folder in Android Studio and run it from there. Build once with the CLI first, and again after you change your app or its plugins: `leptoskit native prepare android` regenerates the Rust inputs without running Gradle.

The `android/` folder is yours: the activity, the manifest, the Gradle files, and `Cargo.toml` for native Rust dependencies such as [runtime plugins](plugins.md). The CLI keeps its generated files in `.leptoskit/android/`.

## Test it

Each project includes an instrumented test that taps **Add one** and waits for **Count: 1**. Update it when you change the home screen, then run it with an emulator running:

```bash
cd android
./gradlew connectedDebugAndroidTest
```

## How it differs from macOS

- The generated app draws seven components: `View`, `Text`, `Button`, `Pressable`, `TextInput`, `ScrollView`, and `Image`. Others, such as `Switch` or `PasswordInput`, stop it with an "unsupported widget" error. The `leptoskit_android` backend crate draws every component; see [generated apps and backend crates](readme.md#generated-apps-and-backend-crates).
- In the backend crate, a `Custom` widget kind with no registered native implementation stops the app. Other platforms render an empty container instead.
- TalkBack gets a smaller set of roles than macOS's accessibility API.
- `Image` loads local files and `android.resource://` URIs, not URLs.
- With the backend crate, the host reports the window insets for the system bars and display cutouts, and the [safe-area utilities](../guide/07-shared-components.md#clear-the-status-bar-and-home-indicator) pad by them.

## Ship an update without the Play Store

The Play Store build carries the Kotlin activity, the interpreter, and a copy of your screens. To change the screens afterwards, publish a code push release for Android:

```bash
leptoskit native bundle --target android --out dist/android
leptoskit codepush release --from dist/android --platform android --channel production
```

The app downloads the release when the user asks for an update, tries it, and keeps it only if it renders. Anything that needs new native code, such as a new plugin or permission, still needs a Play Store release. [Code push](../guide/05-code-push.md) covers signing, publishing, and rollback.
