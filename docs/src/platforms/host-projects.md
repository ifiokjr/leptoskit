# Native application projects

Generate an iOS and Android application:

```sh
leptoskit generate app my-app --platform native
cd my-app
leptoskit native build ios-simulator
leptoskit native build android
```

`leptoskit new` is an alias for app generation. While developing the SDK before publication, pass `--sdk-path /path/to/leptoskit` to use local crates. Native app names use lowercase Cargo package names. Generation refuses an existing destination.

The app contains a Rust/Wasm guest, an installed Rust host library, a UIKit scene, and an Android activity. The sample's buttons exercise a Rust hashing plugin and the platform clipboard plugin. UIKit and Android controls render the guest's tree. `leptoskit build` chooses the iOS simulator on macOS and Android elsewhere for these generated apps; use `native build <target>` to select explicitly.

## Sources and generated state

| Path               | Owner and purpose                                                   |
| ------------------ | ------------------------------------------------------------------- |
| `guest/src/lib.rs` | App-owned Wasm interface and asynchronous plugin calls              |
| `src/lib.rs`       | App-owned Rust static-library entry point                           |
| `hosts/ios/`       | App-owned UIKit scene, Info.plist and XCTest                        |
| `hosts/android/`   | App-owned Gradle app, activity, manifest and instrumentation test   |
| `.leptoskit/`      | Regenerated registrants, Xcode project, assets and native artifacts |
| `leptoskit.toml`   | App identity and selected native plugins                            |

`leptoskit native prepare <target>` regenerates build state and compiles its Rust inputs without running Xcode or Gradle. Run it after changing the guest or plugin dependencies before building from an IDE. Open `.leptoskit/ios/Host.xcodeproj` in Xcode or `hosts/android` in Android Studio. Xcode project edits are regenerated; keep application code and Info.plist changes in `hosts/ios`.

Apple plugins use local Swift Packages. The generated Xcode application target supplies the app bundle and scene lifecycle; a Swift executable package alone does not produce an iOS application. Android uses Gradle's built-in Kotlin support, CMake and the NDK. The checked-in Gradle wrapper verifies its distribution checksum.

## Automatic plugin registration

Add a plugin as a direct Cargo dependency, then select its name:

```toml
[native]
package_id = "com.example.app"
plugins = ["os", "crypto", "clipboard"]
```

The CLI reads each dependency's runtime metadata. Renamed Cargo dependencies are supported. Transitive dependencies are not automatically installed. Duplicate names, missing dependencies and missing target implementations fail generation. Selected plugins receive grants for their declared methods; platform permission dialogs and entitlements remain separate.

Each plugin crate declares either a Rust type implementing `Plugin + Default`:

```toml
[package.metadata.leptoskit.runtime]
name = "example"
rust-type = "Example"
```

Or platform handlers:

```toml
[package.metadata.leptoskit.runtime]
name = "example"
apple-module = "LeptosKitExample"
apple-type = "ExamplePlugin"
android-type = "dev.leptoskit.example.ExamplePlugin"
android-context = true
android-permissions = ["android.permission.VIBRATE"]
```

Apple crates supply `Package.swift` and their exported module. Android handler sources live in the crate's `android/` directory. Rust plugins run synchronously and must remain short and nonblocking; asynchronous native handlers run through the platform dispatcher. Platform calls and interpreter access are confined to the UI thread. Closing the app cancels native tasks and releases the runtime.

## Build and test

iOS needs Xcode and the Rust `aarch64-apple-ios-sim` target on Apple Silicon (`x86_64-apple-ios` on Intel); devices use `aarch64-apple-ios`. Android needs Java 17+, SDK 35, CMake, an installed NDK, `ANDROID_HOME` and the Rust `aarch64-linux-android` or `x86_64-linux-android` target. `ANDROID_NDK_HOME` selects a specific NDK. Use `android` for arm64 devices/emulators, or `android-x86_64` for x86-64 emulators. Preparing a target regenerates Gradle's selected ABI and matching Rust library.

The CLI builds unsigned iOS bundles. Configure development-team signing and provisioning for device installation or distribution. Android debug APKs use Gradle's debug signing; configure release signing in the app before distribution.

Run the generated interaction tests after preparing the target:

```sh
xcodebuild -project .leptoskit/ios/Host.xcodeproj -scheme App \
  -destination 'platform=iOS Simulator,name=iPhone 17 Pro' test
cd hosts/android
./gradlew connectedDebugAndroidTest
```

The tests launch the application, tap the hash and clipboard buttons, and assert visible results. XCTest also verifies relaunch. Xcode retains its `.xcresult`; Gradle retains instrumentation reports under the app's build directory.

## Current limits

The mobile hosts implement views, text, buttons, text fields, scroll containers and file images. Remote images, rich inline text composition, scroll-event forwarding, a device-selection CLI and live attach/reload need further work. The native sample uses the renderer ABI directly; integrating the higher-level Leptos component surface into this scaffold remains separate.

Plugin payloads use the bounded binary bridge. Render transactions currently use versioned JSON and copy into platform objects. This path is not zero-copy and has not established an end-to-end speed advantage over Flutter or React Native. Code-push download/verification is not yet wired into these application hosts.
