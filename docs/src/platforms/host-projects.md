# Native application projects

Create the shared application workspace and build a native destination:

```sh
leptoskit new amazing
cd amazing
leptoskit native build ios-simulator
leptoskit native build android
leptoskit native build macos
leptoskit native build windows
leptoskit native build linux
```

`leptoskit generate app amazing` uses the same generator. Native app names use lowercase Cargo package names. Generation refuses to overwrite existing files.

The counter in `amazing_app` runs in the browser, SSR server, and every native host. Native builds compile that app crate to Wasm and render its shared component tree as platform widgets. The top-level platform folders own startup, packaging, and native dependencies. All six destinations are enabled by default. `leptoskit build --platform <platform>` selects a concrete destination, while `native build <target>` also selects simulator and architecture variants.

## Sources and generated state

| Path                     | Owner and purpose                                                             |
| ------------------------ | ----------------------------------------------------------------------------- |
| `amazing_app/src/`       | App-owned shared UI, routes, state, and services                              |
| `amazing_backend/src/`   | Server-only RPC implementation                                                |
| `amazing_shared/src/`    | Shared response models and domain types                                       |
| `ios/`                   | App-owned UIKit scene, Info.plist, Rust host package, and XCTest              |
| `android/`               | App-owned Gradle app, activity, manifest, Rust host package, and device test  |
| `macos/`                 | App-owned AppKit lifecycle, bundle metadata, and Rust host package            |
| `windows/` and `linux/`  | App-owned desktop runners and their native Cargo dependencies                 |
| `.leptoskit/<platform>/` | Regenerated registrants, build projects, bundled assets, and native artifacts |
| `leptoskit.toml`         | App identity, enabled destinations, and selected native plugins               |

`leptoskit native prepare <target>` regenerates build state and compiles its Rust inputs without running Xcode or Gradle. Run it after changing the app or plugin dependencies before building from an IDE. Open `.leptoskit/ios/Host.xcodeproj` in Xcode or `android/` in Android Studio. Xcode project edits are regenerated. Keep application code and Info.plist changes in `ios/`.

Existing projects that use `guest/`, root `src/lib.rs`, and `hosts/<platform>/` retain that layout. New projects use the shared app crate and top-level platform folders.

Apple plugins use local Swift Packages. The generated Xcode application target supplies the app bundle and scene lifecycle; a Swift executable package alone does not produce an iOS application. Android uses Gradle's built-in Kotlin support, CMake and the NDK. The checked-in Gradle wrapper verifies its distribution checksum.

## Automatic plugin registration

Add a plugin as a direct Cargo dependency of each platform host package that uses it, then select its name in `leptoskit.toml`:

```toml
[native]
package_id = "com.example.app"
plugins = ["os", "crypto", "clipboard"]
```

For example, an iOS host dependency belongs in `ios/Cargo.toml`, in package `amazing_ios`. Keep OS-native libraries out of `amazing_app`, which compiles to Wasm even for native destinations. Application platform features can select optional dependencies that support Wasm. See [platform dependencies](../guide/02-creating.md#add-a-platform-dependency).

The CLI reads each dependency's runtime metadata. Renamed Cargo dependencies are supported. Transitive runtime plugins are not automatically installed. Native support packages in a selected plugin's normal dependency closure are bundled without gaining runtime registration or grants. Duplicate names, missing dependencies, and missing target implementations fail generation. Selected plugins receive grants for their declared methods. Platform permission dialogs and entitlements remain separate.

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

Asynchronous Rust plugins declare `rust-async = true` and a package-relative `rust-factory = "from_config"`. The factory takes `&str` containing JSON host options and returns `Result<Plugin, leptoskit_bridge::Error>`. It must validate its complete configuration before starting any I/O. Async metadata cannot also declare `rust-worker` or `rust-type`; directory factories keep their existing two-directory signature.

Applications grant the plugin and configure its allowed operations separately:

```toml
[native]
plugins = ["http"]

[native.plugin-options.http]
origins = ["https://api.example.com"]
```

Each selected async plugin requires its own options table. Unknown option tables, options supplied to a plugin that does not accept them, and options larger than 64 KiB are rejected. The plugin factory validates its schema and scope rules; a missing or invalid scope never enables unrestricted access. These values are embedded in the native registrant, not supplied by guest requests, and should contain policy rather than secrets.

`leptoskit native prepare` and `native build` enable `async-host` on the selected async plugin dependencies and bridge only for the host Cargo build. They register the factory result with `app.install_async(Arc::new(...))`. The separate guest build receives no async host features or runtime dependency.

Blocking plugin factories receive the application's data and cache directories and return `Result<impl Plugin + Send, Error>`. The generated host retains the plugin and request bytes independently of its interpreter, executes blocking work on a worker, and completes the owned reply on the UI thread. Cancelling queued work prevents invocation; work already inside an OS call may finish, so a cancellation is not a rollback of side effects.

Apple crates supply `Package.swift` and their exported module. Android handler sources live in the crate's `android/` directory. Shared native implementation crates declare `[package.metadata.leptoskit.native-support]` with `apple-module` and/or `android = true`. They are copied from the selected normal dependency graph. `android-query-actions` declares package-visibility intent actions; generated XML preserves the application's own queries and permissions.

Presentation plugins additionally declare `apple-context = true`. Their initializer receives `NativePluginContext`, which weakly references the exact UIKit/AppKit owner. The document picker uses that owner for presentation; screen orientation uses the owning iOS scene and supported orientation mask. Generated hosts supply the context to registration. Custom hosts must supply an appropriate owner and preserve the same lifecycle contract.

The generated Android Activity implements `NativeActivityResultHost`, forwards external results and saves the broker's request-code watermark across recreation. Only one external presentation is active. Cancelling a caller retains that presentation's slot until its result returns, so a late callback cannot finish a replacement request. Host destruction cancels outstanding callers. Document import uses this broker without requesting storage permission or retaining permanent document access.

Select `document-picker`, `screen-orientation`, `light_sensor` and `pedometer` with their matching direct crate dependencies. Document import and orientation have browser adapters; native desktop orientation, Apple ambient light and desktop/browser pedometers are unavailable. Pedometer needs app-owned `NSMotionUsageDescription` on iOS and `android.permission.ACTIVITY_RECOGNITION` on Android; selection does not generate either declaration or grant runtime permission. Sandboxed macOS document import needs app-owned user-selected read access.

The generated Android manifest handles `orientation|screenSize` configuration changes and the host resizes its rendering surface. This keeps an orientation request from recreating the Activity and restarting its guest. Custom hosts need equivalent handling. A lease requests policy rather than promising rotation; large-screen Android and multitasking Apple configurations may ignore it.

Ordinary Rust plugins run synchronously and must remain short and nonblocking. Swift and Kotlin handlers suspend or dispatch blocking work explicitly. Interpreter access stays on the UI thread. Hosts retain cancelled jobs until native work releases its resources, limit outstanding jobs to 128 and retained request payloads to 32 MiB, and coalesce pump scheduling. These limits do not bound arbitrary allocations made inside a plugin.

## Build and test

iOS needs Xcode and the Rust `aarch64-apple-ios-sim` target on Apple Silicon (`x86_64-apple-ios` on Intel); devices use `aarch64-apple-ios`. macOS uses the host architecture and Swift SDK to assemble `.leptoskit/macos/App.app`. Android needs Java 17+, SDK 35, CMake, an installed NDK, `ANDROID_HOME` and the Rust `aarch64-linux-android` or `x86_64-linux-android` target. `ANDROID_NDK_HOME` selects a specific NDK. The CLI configures the NDK C/C++ compiler and archiver as well as the Rust linker, so plugins with bundled native libraries use the same target. Use `android` for arm64 devices/emulators, or `android-x86_64` for x86-64 emulators. Preparing a target regenerates Gradle's selected ABI and matching Rust library.

The CLI ad-hoc signs iOS simulator applications so Keychain access works without a development certificate. Device bundles remain unsigned; configure development-team signing and provisioning for device installation or distribution. Android debug APKs use Gradle's debug signing; configure release signing in the app before distribution.

Native builds use a stable subdirectory of Cargo's configured target directory, keyed by the canonical application path and native target. Guest and host artifacts are both built and copied from that directory. This preserves incremental builds while preventing applications with identical crate names from substituting each other's binaries when they share `CARGO_TARGET_DIR`.

Only one `native prepare` or `native build` command may run for an application at a time. The CLI holds an operating-system file lock through generation, compilation, and platform bundling; another command for the same application fails with a clear message. Different applications may build concurrently. When building directly in Xcode or Android Studio, finish `native prepare` first and avoid preparing another target until that build finishes.

Run the generated interaction tests after preparing the target:

```sh
xcodebuild -project .leptoskit/ios/Host.xcodeproj -scheme App \
  -destination 'platform=iOS Simulator,name=iPhone 17 Pro' \
  CODE_SIGNING_ALLOWED=YES CODE_SIGN_IDENTITY=- test
cd android
./gradlew connectedDebugAndroidTest
```

The generated tests launch the shared counter, tap **Add one**, and wait for **Count: 1**. Xcode retains its `.xcresult`. Gradle retains instrumentation reports under the app's build directory. Older native plugin examples have additional plugin and relaunch checks.

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
leptoskit native bundle --target ios --out dist/ios
leptoskit codepush release --from dist/ios --channel production --signing-key ~/.config/leptoskit/codepush.key
```

Then upload the channel with `leptoskit codepush publish`, or serve it with `leptoskit codepush serve`; see [code push](../guide/05-code-push.md).

`native bundle` compiles the app for `wasm32-unknown-unknown` without JavaScript glue and writes `app.wasm` next to `runtime.json`. Select a target explicitly when `app.default_platform` is `web` or `server`. Each native destination selects its own app feature and file overrides, so publish its bundle separately.

`runtime.json` names the Wasm ABI, native contract, and required plugins. Shared app workspaces append the destination to `native.update_contract`, for example `mobile-v1/ios`. A host rejects bundles for another destination. The release signature covers `runtime.json` as an ordinary bundle file.

`native prepare` packs the same bundle into the app as its store baseline (`app.lkb`), along with the trusted keys and server in `codepush.json`. The app ships the guest only inside `app.lkb`. With code push off, or if the update store fails, the host reads its Wasm from there after checking every file hash.

When code push is on, the host shows an update bar above the guest:

1. **Check for update** downloads the newest release on a background worker. Rust verifies the signature, app, channel, and version, then checks that `runtime.json` matches the installed shell's ABI, contract, and plugins. The verified container and its signature are written to the app's private, non-backed-up storage. Nothing runs yet.
2. **Activate update** starts the downloaded guest in a new native view while the current one stays on screen. The release is a trial until its native root has attached and survived a UI turn; only then does Rust confirm it and the host swap views.
3. If the trial traps, the host rejects it and keeps the working guest running. A rejected release is refused if the registry offers it again. If the process dies during a trial, or the trial never renders, the next launch rejects that release and starts the last confirmed one.
4. Every launch loads the confirmed release from disk and verifies its signature again, so it works offline. A new store build supersedes every earlier download, even at the same app version. Only the working release and a staged one are kept on disk.

Release builds only reach `https://` registries. Debug builds can opt into a local `http://` registry with `codepush.allow_http = true`. The host follows no redirects, caps responses and unpacked bundles at 32 MiB, and gives a connection 15 seconds, a response 30 seconds, and a whole download 10 minutes. A 404 for the channel's `index.json` means there is no release yet. If the update store itself fails, the host runs the store-shipped guest and reports that updates are unavailable.

## Current limits

The mobile hosts implement views, text, buttons, text fields, scroll containers, and file images. Remote images, rich inline text composition, scroll-event forwarding, a device-selection CLI, and live attach/reload need further work. The shared starter uses the SDK's native guest adapter to render the same application tree on each platform.

Plugin payloads use the bounded binary bridge. Render transactions currently use versioned JSON and copy into platform objects. This path is not zero-copy and has not established an end-to-end speed advantage over Flutter or React Native.

Code push establishes startup health only: a guest that renders its first frame and then fails later is not rolled back automatically. The hosts check for updates only when the user asks, and `codepush.mandatory` is not read yet. A guest that needs a new plugin, permission, or ABI needs a store release with a new `update_contract`. Store review rules still apply to what downloaded code changes, so a large product change can need review even when the shell is unchanged.

## HarmonyOS hosts

`leptoskit new my_app --platform native --native-target harmonyos` generates an experimental ArkUI counter and a DevEco Stage-model project. `leptoskit native build harmonyos` produces an arm64 HAP; `harmonyos-x86_64` selects the x86-64 emulator ABI. See [HarmonyOS](harmonyos.md) for SDK setup, signing, supported widgets, and validation limits.
