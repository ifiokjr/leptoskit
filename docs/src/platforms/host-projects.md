# Host projects

Your app's screens compile to WebAssembly. To run on a phone or a desktop, that WebAssembly needs a native app around it: something that opens a window, starts the interpreter, draws the widgets, and asks the operating system for permissions. leptoskit calls that app the **host**. `leptoskit new` generates one host per platform, in its own folder, and they all run the same `<name>_app` crate.

## What lives where

For a project named `water`:

| Path                     | What it holds                                                                                     | Who edits it |
| ------------------------ | ------------------------------------------------------------------------------------------------- | ------------ |
| `water_app/src/`         | Your screens, routes, state, and services, shared by every platform                               | You          |
| `water_backend/src/`     | Server-only code, such as RPC handlers                                                            | You          |
| `water_shared/src/`      | Types the app and the backend both use                                                            | You          |
| `ios/`                   | The UIKit app: `App.swift`, `Info.plist`, `Cargo.toml` for native dependencies, and a UI test     | You          |
| `android/`               | The Android app: Gradle files, the activity, the manifest, `Cargo.toml`, and an instrumented test | You          |
| `macos/`                 | The AppKit app: `App.swift`, `Info.plist`, and `Cargo.toml`                                       | You          |
| `windows/` and `linux/`  | Small Rust programs that open a window and run your app, with their `Cargo.toml`                  | You          |
| `leptoskit.toml`         | The app's name and id, its enabled platforms, and its runtime plugins                             | You          |
| `.leptoskit/<platform>/` | Generated build projects, plugin registration code, bundled assets, and build output              | The CLI      |

Everything under `.leptoskit/` is regenerated on every build, including the Xcode project at `.leptoskit/ios/Host.xcodeproj`, so make changes in the platform folders. Your app's own code stays in `water_app`: it compiles to WebAssembly for every native platform, so code that calls UIKit, Android's Java APIs, or other operating system libraries belongs in a host folder or in a [runtime plugin](plugins.md).

Projects created before this layout, with `guest/`, a root `src/lib.rs`, and `hosts/<platform>/`, keep working as they are.

## What each platform needs

| Platform | Tools                                                                                           | Rust targets                                                               |
| -------- | ----------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------- |
| iOS      | A Mac with Xcode and a simulator runtime                                                        | `aarch64-apple-ios-sim` (`x86_64-apple-ios` on Intel), `aarch64-apple-ios` |
| macOS    | A Mac with Xcode or its command line tools                                                      | Your Mac's architecture                                                    |
| Android  | Java 17+, Android SDK 35, CMake, an NDK, and `ANDROID_HOME` (`ANDROID_NDK_HOME` to pick an NDK) | `aarch64-linux-android` or `x86_64-linux-android`                          |
| Windows  | Windows with the MSVC toolchain and the Windows SDK                                             | Your machine's architecture                                                |
| Linux    | Linux with GTK 4.6+ development files and `pkg-config`                                          | Your machine's architecture                                                |

Every platform also needs `wasm32-unknown-unknown` for your app. The CLI configures the NDK's compilers for Android itself, so plugins with bundled C or C++ libraries build for the same target.

iOS simulator builds are signed ad hoc, so Keychain access works without a development certificate. iOS device builds aren't signed: set your development team in Xcode. Android debug APKs use Gradle's debug key; configure release signing in `android/app/build.gradle.kts` before distributing.

## Build from Xcode or Android Studio

`leptoskit native build <target>` does everything: it compiles your app to WebAssembly, generates the host project, and runs Xcode or Gradle. To work in an IDE instead, run `leptoskit native prepare <target>` first. It regenerates the host project and compiles the Rust parts without running Xcode or Gradle. Then open `.leptoskit/ios/Host.xcodeproj` in Xcode, or the `android/` folder in Android Studio. Prepare again after you change your app or its plugins.

Only one `native prepare` or `native build` can run for the same project at a time; a second one stops with a message. Don't prepare another target while Xcode or Android Studio is building.

## Test the generated apps

The iOS and Android hosts each include a UI test that launches the app, taps **Add one**, and waits for **Count: 1**. Update them when you change the home screen.

```sh
xcodebuild -project .leptoskit/ios/Host.xcodeproj -scheme App \
  -destination 'platform=iOS Simulator,name=iPhone 17 Pro' \
  CODE_SIGNING_ALLOWED=YES CODE_SIGN_IDENTITY=- test
cd android
./gradlew connectedDebugAndroidTest
```

Xcode keeps its `.xcresult`, and Gradle keeps its reports under `android/app/build`.

## Automatic plugin registration

A runtime plugin gives your app an operating-system service, such as the clipboard or location. Using one takes two steps:

1. Add the plugin crate as a dependency of each host that should have it. For iOS that is `ios/Cargo.toml`, not `water_app/Cargo.toml`.
2. List its name in `leptoskit.toml`:

```toml
[native]
plugins = ["os", "crypto", "clipboard"]
```

At build time, the CLI reads each listed dependency's metadata and generates the code that registers it with the host and grants your app access to its methods. Renamed dependencies work. A plugin that is only a dependency of another dependency isn't registered. A missing dependency or a duplicate name stops the build with an error.

### Plugins a platform can't run

One `leptoskit.toml` serves every platform, even when a plugin doesn't exist everywhere. When a listed plugin has no implementation for the platform being built, or that platform's app can't run it yet, the build leaves it out and warns:

```text
warning: haptics has no Linux implementation; calls fail with Unavailable
```

The app still builds. On that platform, creating the plugin's client or calling it fails with `Error::Unavailable`, the same error a plugin returns for anything else the device can't do, so handle it where you call the plugin. Today:

- the generated Windows, Linux, and terminal apps leave every runtime plugin out;
- HarmonyOS apps run synchronous Rust plugins, such as `os`, and ArkTS plugins, such as `posture`. They leave out plugins that run on a worker thread or on the async host, such as `crypto`, `http`, `shell`, and `websocket`, and app-owned services;
- iOS, Android, and macOS leave out only plugins that have no implementation for them.

A plugin can also declare that a platform doesn't have the service at all; the build leaves it out there without a warning. To stop the build instead whenever a listed plugin is left out, including those, set:

```toml
[native]
missing-plugins = "error"
```

Registering a plugin doesn't grant operating-system permissions. Add the permission strings and entitlements the plugin's README asks for to your host, such as `NSMotionUsageDescription` in `ios/Info.plist` for the pedometer, or `android.permission.ACTIVITY_RECOGNITION` in the Android manifest. Your app still asks the user at run time.

Network and process plugins also need a policy that says what they may reach. You give it in `leptoskit.toml`, and the host enforces it, not your app's code:

```toml
[native]
plugins = ["http"]

[native.plugin-options.http]
origins = ["https://api.example.com"]
```

Each plugin that takes options needs its own table. Unknown tables, options for a plugin that takes none, and tables over 64 KiB are rejected. A missing or invalid policy never means "allow everything". The options are compiled into the host, so keep secrets out of them.

The [plugin support matrix](plugin-inventory.md) lists every plugin and the platforms it supports.

## Plugin metadata reference

This section is for plugin authors. A plugin crate tells the CLI how to register it in `[package.metadata.leptoskit.runtime]`, in one of these forms:

| Kind                      | Metadata                                                                                                 | The host…                                                                                                                                                                                                 |
| ------------------------- | -------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Quick Rust handler        | `rust-type = "Example"`, a type implementing `Plugin + Default`                                          | calls it on the thread that runs your screens: its own thread on iOS and Android, the UI thread elsewhere. It must return quickly; a handler that needs UIKit or Android views belongs in Swift or Kotlin |
| Blocking Rust handler     | `rust-factory = "from_app_dirs"` and `rust-worker = true`                                                | calls the factory with the app's data and cache directories, then runs each call on a worker thread                                                                                                       |
| Async Rust handler        | `rust-async = true` and `rust-factory = "from_config"`                                                   | calls the factory with the app's options as JSON, and awaits each call without blocking the interpreter                                                                                                   |
| Swift and Kotlin handlers | `apple-module`, `apple-type`, `android-type`, and optionally `android-context` and `android-permissions` | registers the Swift class on Apple platforms and the Kotlin class on Android                                                                                                                              |
| ArkTS handler             | `harmonyos-type = "ExamplePlugin"`, and optionally `harmonyos-context`                                   | constructs the ArkTS class on HarmonyOS                                                                                                                                                                   |
| App-owned async service   | `rust-service-factory = "native::from_app_dirs"`                                                         | runs one long-lived service for the app; see [runtime plugins](plugins.md#application-owned-asynchronous-services)                                                                                        |

For example, a plugin with Swift and Kotlin handlers:

```toml
[package.metadata.leptoskit.runtime]
name = "example"
apple-module = "LeptosKitExample"
apple-type = "ExamplePlugin"
android-type = "dev.leptoskit.example.ExamplePlugin"
android-context = true
android-permissions = ["android.permission.VIBRATE"]
```

Apple plugins ship a `Package.swift` exporting the module, and declare `apple-module` and `apple-type` together; Android sources live in the crate's `android/` directory. HarmonyOS sources live in `harmonyos/`: `harmonyos-type = "ExamplePlugin"` names the class that `harmonyos/ExamplePlugin.ts` exports, which implements the host's `NativePlugin` interface from `../plugins`. With `harmonyos-context = true` its constructor receives the ability's `common.Context`. Every `.ts` file there except `*_test.ts` is copied into the app, so keep test fakes in a subdirectory. A crate of shared native code that several plugins use declares `[package.metadata.leptoskit.native-support]` with `apple-module`, `android = true`, or both. `android-query-actions` declares the intent actions a plugin needs to see other apps; the generated manifest keeps your own queries and permissions.

On Android, a plugin that opens a URI needs a query for its scheme as well as its action. Pair them in the plugin's runtime metadata so installed apps with matching intent filters are visible:

```toml
android-query-intents = [
	{ action = "android.intent.action.VIEW", scheme = "solana-wallet" },
]
```

The generated manifest adds the action and scheme together in one `<intent>`. An existing action-only query does not replace this declaration. Your own queries remain in place, including queries for particular hosts or categories.

### Per-platform registration

A plugin often has a Swift and a Kotlin implementation for phones and a Rust one for the desktop. Say which platform uses which in a table named after the platform:

```toml
[package.metadata.leptoskit.runtime]
name = "clipboard"
apple-module = "LeptosKitClipboard"
apple-type = "ClipboardPlugin"
android-type = "dev.leptoskit.plugins.clipboard.ClipboardPlugin"
android-context = true

[package.metadata.leptoskit.runtime.windows]
rust-factory = "desktop::from_context"

[package.metadata.leptoskit.runtime.linux]
rust-factory = "desktop::from_context"
```

The tables are `ios`, `macos`, `android`, `harmonyos`, `windows`, `linux`, and `tui` (the terminal). Each takes `rust-type` or `rust-factory`, with `rust-worker` or `rust-async` as in the shared fields, or only `unavailable = true`, which says the platform doesn't have the service at all: the plugin is left out there without a warning. Any other key or table name is an error.

For each platform, the CLI uses the first of these that the plugin declares:

1. the platform's table;
2. the shared `rust-type` or `rust-factory`, which apply to every platform;
3. the platform's native handler: `apple-type` on iOS and macOS, `android-type` on Android, `harmonyos-type` on HarmonyOS.

If the platform's app can't run the Rust handler, for example a worker plugin on HarmonyOS, the native handler is used when there is one. Otherwise the plugin is [left out with a warning](#plugins-a-platform-cant-run). A plugin with an app-owned service can't declare Rust handlers in platform tables.

An async factory takes the options as a `&str` of JSON and returns `Result<Plugin, leptoskit_bridge::Error>`; it must reject invalid options before starting any I/O. A blocking factory returns `Result<impl Plugin + Send, Error>`. Cancelling a queued call means it never runs, but a call already inside the operating system may still finish, so cancellation doesn't undo side effects.

Plugins that present UI, such as the document picker, declare `apple-context = true` and receive a `NativePluginContext` that weakly references the UIKit or AppKit object that owns them. On Android, the generated activity implements `NativeActivityResultHost`, which routes results from other activities back to the right request, even across activity recreation. The generated Android manifest also handles orientation, screen-size, and screen-layout changes itself, so rotating the phone or folding a foldable resizes your app instead of restarting it. A custom host has to provide all three.

## Code push in the generated apps

The iOS and Android hosts can replace your app's WebAssembly over the air. The host itself, the interpreter, the widgets, and the plugins only change through a store release. The generated macOS, Windows, and Linux apps don't check for updates yet.

Turn it on in `leptoskit.toml`:

```toml
[native]
update_contract = "mobile-v1"

[codepush]
enabled = true
server = "https://updates.example.com"
channel = "production"
public_keys = ["<hex public key from leptoskit codepush keygen>"]
```

Then build, sign, and publish a release for each platform:

```sh
leptoskit native bundle --target ios --out dist/ios
leptoskit codepush release --from dist/ios --platform ios --channel production --signing-key ~/.config/leptoskit/codepush.key
leptoskit codepush publish --platform ios --endpoint https://s3.us-west-004.backblazeb2.com --bucket my-app-updates
```

`native bundle` writes `app.wasm` and `runtime.json`, which names the host interface, the update contract, and the plugins the release needs. Each platform builds its own bundle, because each turns on its own platform feature and per-platform files. The contract gets the platform appended, such as `mobile-v1/ios`, so an iOS host refuses an Android release. [Code push](../guide/05-code-push.md) covers keys, hosting, and publishing.

The store build carries a first copy of your app, `app.lkb`, with the trusted keys and server in `codepush.json`. When code push is on, the host shows an update bar above your app:

1. **Check for update** downloads the newest release in the background. The host checks its signature, app, channel, and version, and checks that `runtime.json` matches its own interface and plugins. Nothing runs yet.
2. **Activate update** starts the new release next to the current one. It is a trial until it has drawn its first screen and survived a moment on the UI thread; then the host switches to it.
3. If the trial crashes, the host keeps the current release and refuses that release from then on. If the app is killed during a trial, or the trial never draws, the next launch goes back to the last good release.
4. Every launch verifies the release's signature again, and works offline. A new store build replaces every earlier download.

Release builds only reach `https://` servers; debug builds can use a local `http://` server with `codepush.allow_http = true`. Downloads follow no redirects, are capped at 32 MiB, and time out after 15 seconds to connect, 30 seconds to respond, and 10 minutes in total. If the update store fails, the host runs the copy it shipped with and says updates are unavailable.

Code push protects startup only: a release that draws its first screen and fails later is not rolled back automatically. Updates are checked only when the user taps the button, and `codepush.mandatory` isn't read yet. A release that needs a new plugin, permission, or host interface needs a store release with a new `update_contract`. Store review rules still apply to what a download changes.

## HarmonyOS

HarmonyOS uses its own starter, with `guest/` and `hosts/harmonyos/` instead of the layout above: `leptoskit new my_app --platform harmonyos`. `leptoskit native build harmonyos` produces an arm64 HAP, and `harmonyos-x86_64` builds for x86-64 emulators. See [HarmonyOS](harmonyos.md).
