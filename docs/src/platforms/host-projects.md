# Native application projects

Generate iOS, Android and macOS applications:

```sh
leptoskit generate app my-app --platform native
cd my-app
leptoskit native build ios-simulator
leptoskit native build android
leptoskit native build macos
```

`leptoskit new` is an alias for app generation. While developing the SDK before publication, pass `--sdk-path /path/to/leptoskit` to use local crates. Native app names use lowercase Cargo package names. Generation refuses an existing destination.

The app contains a Rust/Wasm guest, an installed Rust host library, a UIKit scene, an Android activity, and an AppKit application. The sample exercises hashing, clipboard, haptics, battery, device information, secure storage and application files. Native controls render the guest's tree. `leptoskit build` chooses macOS when the generated AppKit host exists on a Mac, and Android elsewhere; use `native build <target>` to select explicitly.

## Sources and generated state

| Path               | Owner and purpose                                                   |
| ------------------ | ------------------------------------------------------------------- |
| `guest/src/lib.rs` | App-owned Wasm interface and asynchronous plugin calls              |
| `src/lib.rs`       | App-owned Rust static-library entry point                           |
| `hosts/ios/`       | App-owned UIKit scene, Info.plist and XCTest                        |
| `hosts/android/`   | App-owned Gradle app, activity, manifest and instrumentation test   |
| `hosts/macos/`     | App-owned AppKit application delegate, menus and bundle metadata    |
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

The CLI reads each dependency's runtime metadata. Renamed Cargo dependencies are supported. Transitive runtime plugins are not automatically installed. Native support packages in a selected plugin's normal dependency closure are bundled without gaining runtime registration or grants. Duplicate names, missing dependencies and missing target implementations fail generation. Selected plugins receive grants for their declared methods; platform permission dialogs and entitlements remain separate.

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

Blocking Rust plugins use a factory and worker registration:

```toml
[package.metadata.leptoskit.runtime]
name = "filesystem"
rust-factory = "from_app_dirs"
rust-worker = true
```

The factory receives the application's data and cache directories and returns `Result<impl Plugin + Send, Error>`. The generated host retains the plugin and request bytes independently of its interpreter, executes blocking work on a worker, and completes the owned reply on the UI thread. Cancelling queued work prevents invocation; work already inside an OS call may finish, so a cancellation is not a rollback of side effects.

Apple crates supply `Package.swift` and their exported module. Android handler sources live in the crate's `android/` directory. Shared native implementation crates declare `[package.metadata.leptoskit.native-support]` with `apple-module` and/or `android = true`. They are copied from the selected normal dependency graph. `android-query-actions` declares package-visibility intent actions; generated XML preserves the application's own queries and permissions.

Ordinary Rust plugins run synchronously and must remain short and nonblocking. Swift and Kotlin handlers suspend or dispatch blocking work explicitly. Interpreter access stays on the UI thread. Hosts retain cancelled jobs until native work releases its resources, limit outstanding jobs to 128 and retained request payloads to 32 MiB, and coalesce pump scheduling. These limits do not bound arbitrary allocations made inside a plugin.

## Build and test

iOS needs Xcode and the Rust `aarch64-apple-ios-sim` target on Apple Silicon (`x86_64-apple-ios` on Intel); devices use `aarch64-apple-ios`. macOS uses the host architecture and Swift SDK to assemble `.leptoskit/macos/App.app`. Android needs Java 17+, SDK 35, CMake, an installed NDK, `ANDROID_HOME` and the Rust `aarch64-linux-android` or `x86_64-linux-android` target. `ANDROID_NDK_HOME` selects a specific NDK. The CLI configures the NDK C/C++ compiler and archiver as well as the Rust linker, so plugins with bundled native libraries use the same target. Use `android` for arm64 devices/emulators, or `android-x86_64` for x86-64 emulators. Preparing a target regenerates Gradle's selected ABI and matching Rust library.

The CLI ad-hoc signs iOS simulator applications so Keychain access works without a development certificate. Device bundles remain unsigned; configure development-team signing and provisioning for device installation or distribution. Android debug APKs use Gradle's debug signing; configure release signing in the app before distribution.

Run the generated interaction tests after preparing the target:

```sh
xcodebuild -project .leptoskit/ios/Host.xcodeproj -scheme App \
  -destination 'platform=iOS Simulator,name=iPhone 17 Pro' \
  CODE_SIGNING_ALLOWED=YES CODE_SIGN_IDENTITY=- test
cd hosts/android
./gradlew connectedDebugAndroidTest
```

The tests launch the application and exercise hashing, clipboard, haptics requests, battery and device readings, secure storage, and application files. XCTest also verifies relaunch. Xcode retains its `.xcresult`; Gradle retains instrumentation reports under the app's build directory.

## Code push

The generated hosts can replace the Wasm guest over the air. Native code (the shell, interpreter, widgets, and plugins) only changes through a store release. Turn it on in `leptoskit.toml`:

```toml
[native]
update_contract = "mobile-v1"

[codepush]
enabled = true
server = "https://updates.example.com"
channel = "production"
public_keys = ["<hex public key from leptoskit codepush keygen>"]
```

Build the guest on its own and publish it:

```sh
leptoskit native bundle --out dist
leptoskit codepush release --from dist --channel production --signing-key ~/.config/leptoskit/codepush.key
```

Then upload the channel with `leptoskit codepush publish`, or serve it with `leptoskit codepush serve`; see [code push](../guide/05-code-push.md).

`native bundle` compiles the guest for `wasm32-unknown-unknown` without JavaScript glue and writes `app.wasm` next to a `runtime.json` that names the Wasm ABI, `native.update_contract`, and the plugins the guest needs. `runtime.json` is an ordinary bundle file, so the release signature covers it. `native prepare` packs the same bundle into the app as its store baseline (`app.lkb`), along with the trusted keys and server in `codepush.json`. The app ships the guest only inside `app.lkb`. With code push off, or if the update store fails, the host reads its Wasm from there after checking every file hash.

When code push is on, the host shows an update bar above the guest:

1. **Check for update** downloads the newest release on a background worker. Rust verifies the signature, app, channel, and version, then checks that `runtime.json` matches the installed shell's ABI, contract, and plugins. The verified container and its signature are written to the app's private, non-backed-up storage. Nothing runs yet.
2. **Activate update** starts the downloaded guest in a new native view while the current one stays on screen. The release is a trial until its native root has attached and survived a UI turn; only then does Rust confirm it and the host swap views.
3. If the trial traps, the host rejects it and keeps the working guest running. A rejected release is refused if the registry offers it again. If the process dies during a trial, or the trial never renders, the next launch rejects that release and starts the last confirmed one.
4. Every launch loads the confirmed release from disk and verifies its signature again, so it works offline. A new store build supersedes every earlier download, even at the same app version. Only the working release and a staged one are kept on disk.

Release builds only reach `https://` registries. Debug builds can opt into a local `http://` registry with `codepush.allow_http = true`. The host follows no redirects, caps responses and unpacked bundles at 32 MiB, and gives a connection 15 seconds, a response 30 seconds, and a whole download 10 minutes. A 404 for the channel's `index.json` means there is no release yet. If the update store itself fails, the host runs the store-shipped guest and reports that updates are unavailable.

`cargo run -p leptoskit_cli --example mobile_codepush -- --platform ios|android --device <id> --sdk-path . --cli target/debug/leptoskit` runs the full journey against a booted simulator or emulator, and CI runs it on both platforms. It scaffolds an app, builds four signed guests from one source, and serves them from a local registry. Then it taps a counter that steps by one, downloads and activates a replacement that steps by ten, and relaunches offline. Then it activates a guest that traps on startup and checks that the working guest is still interactive, both straight away and after a relaunch. Finally it installs a second good update after that rollback, and checks that it survives an offline relaunch.

## Current limits

The mobile hosts implement views, text, buttons, text fields, scroll containers and file images. Remote images, rich inline text composition, scroll-event forwarding, a device-selection CLI and live attach/reload need further work. The native sample uses the renderer ABI directly; integrating the higher-level Leptos component surface into this scaffold remains separate.

Plugin payloads use the bounded binary bridge. Render transactions currently use versioned JSON and copy into platform objects. This path is not zero-copy and has not established an end-to-end speed advantage over Flutter or React Native.

Code push establishes startup health only: a guest that renders its first frame and then fails later is not rolled back automatically. The hosts check for updates only when the user asks, and `codepush.mandatory` is not read yet. A guest that needs a new plugin, permission, or ABI needs a store release with a new `update_contract`. Store review rules still apply to what downloaded code changes, so a large product change can need review even when the shell is unchanged.

## HarmonyOS hosts

`leptoskit new my_app --platform native --native-target harmonyos` generates an experimental ArkUI counter and a DevEco Stage-model project. `leptoskit native build harmonyos` produces an arm64 HAP; `harmonyos-x86_64` selects the x86-64 emulator ABI. See [HarmonyOS](harmonyos.md) for SDK setup, signing, supported widgets, and validation limits.
