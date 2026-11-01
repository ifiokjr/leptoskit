# Android

On Android, your screens are drawn with Android views: a `Button` is an `android.widget.Button`, a `TextInput` an `EditText`, a `ScrollView` a `ScrollView`. The app that `leptoskit new` generates is a small Kotlin activity that runs your screens as WebAssembly, so it can also replace them over the air with [code push](../guide/05-code-push.md). Text is measured with the same fonts Android draws it with, so each label and button gets exactly the space it needs.

The generated app runs your screens' code on a thread of its own, not Android's main thread. Updating state, laying the screen out, and measuring text happen there, and the main thread only applies the result to the views. A slow update delays what you draw next, but scrolling, animations, and touches keep running meanwhile.

## What you need

- Java 17 or newer, the Android SDK with platform 35, CMake, and an NDK. Set `ANDROID_HOME` to the SDK; `ANDROID_NDK_HOME` picks a specific NDK.
- The Rust targets for your device or emulator, and for WebAssembly:

```bash
rustup target add aarch64-linux-android x86_64-linux-android wasm32-unknown-unknown
```

## Develop with live reload

```bash
leptoskit dev --platform android
```

`dev` builds the app, installs it on every attached phone and emulator, and opens it. With nothing attached, it boots your first emulator and leaves it running for next time. When you save, it rebuilds your interface and the running app swaps it in. It finds `adb` and the emulator through `ANDROID_HOME`. [Building and serving](../guide/04-building.md#leptoskit-dev) covers the rest.

## Build and run

```bash
leptoskit build --platform android
```

This builds an arm64 debug APK, for phones and for emulators on Apple silicon, at `android/app/build/outputs/apk/debug/app-debug.apk`. Install it on a running emulator or a connected phone:

```bash
adb install -r android/app/build/outputs/apk/debug/app-debug.apk
```

For an x86-64 emulator, build `leptoskit native build android-x86_64` instead. Add `--release` for a release APK, and an App Bundle beside it for Google Play. Release builds are signed with your upload key when the `LEPTOSKIT_ANDROID_KEYSTORE`, `LEPTOSKIT_ANDROID_KEYSTORE_PASSWORD`, `LEPTOSKIT_ANDROID_KEY_ALIAS`, and `LEPTOSKIT_ANDROID_KEY_PASSWORD` variables are set, and unsigned when they aren't. `leptoskit publish play` uploads to Google Play; see [Publishing to the stores](../guide/24-publishing.md#google-play).

You can also open the `android/` folder in Android Studio and run it from there. Build once with the CLI first, and again after you change your app or its plugins: `leptoskit native prepare android` regenerates the Rust inputs without running Gradle.

The `android/` folder is yours: the activity, the manifest, the Gradle files, and `Cargo.toml` for native Rust dependencies such as [runtime plugins](plugins.md). The CLI keeps its generated files in `.leptoskit/android/`, including the R8 keep rules that release builds need for the native library; put rules for your own code in `android/app/proguard-rules.pro`.

## Test it

Each project includes an instrumented test that taps **Add one** and waits for **Count: 1**. Update it when you change the home screen, then run it with an emulator running:

```bash
cd android
./gradlew connectedDebugAndroidTest
```

## How it differs from macOS

- The generated app draws nine components: `View`, `Text`, `Button`, `Pressable`, `TextInput`, `ScrollView`, `Image`, `Vector`, and `Rive`. Others, such as `Switch` or `PasswordInput`, stop it with an "unsupported widget" error. The `leptoskit_android` backend crate draws every component; see [generated apps and backend crates](readme.md#generated-apps-and-backend-crates).
- [Rive](../guide/20-rive.md) uploads native vector frames to a reusable Android bitmap. The generated host schedules through Choreographer, stops when idle or offscreen, and treats disabled system animations as reduced motion. Native playback rejects text, image, layout, script, and audio assets.
- In the backend crate, a `Custom` widget kind with no registered native implementation stops the app. Other platforms render an empty container instead.
- TalkBack gets a smaller set of roles than macOS's accessibility API.
- [Sign-in](../guide/23-sign-in.md#native-apps) uses Credential Manager for passkeys once your site vouches for the app's signing certificate. Other providers open in Chrome's Auth Tab, which needs Chrome 137 or later as the default browser; with another default browser, provider sign-in ends with the outcome `unsupported`. Sign in with Apple uses Apple's web sign-in, in the same Auth Tab.
- `Image` loads local files and `android.resource://` URIs, not URLs.
- The generated app keeps your screen clear of the status and navigation bars. The bars take your root view's background, as a browser paints the page's background across the whole window, with light icons on a dark one. A root without an opaque background leaves the theme's white bars.
- With the backend crate, the host reports the window insets for the system bars and display cutouts, and the [safe-area utilities](../guide/07-shared-components.md#clear-the-status-bar-and-home-indicator) pad by them.

## Ship an update without the Play Store

The Play Store build carries the Kotlin activity, the interpreter, and a copy of your screens. To change the screens afterwards, publish a code push release for Android:

```bash
leptoskit native bundle --target android --out dist/android
leptoskit codepush release --from dist/android --platform android --channel production
```

The app downloads the release when the user asks for an update, tries it, and keeps it only if it renders. Anything that needs new native code, such as a new plugin or permission, still needs a Play Store release. [Code push](../guide/05-code-push.md) covers signing, publishing, and rollback.
