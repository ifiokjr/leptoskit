# Runtime plugins

Runtime plugins give an installed LeptosKit host access to operating-system services. They are separate from `leptoskit_plugin`, whose hooks customize builds. Each runtime plugin is an independent Cargo crate. Adding a dependency does not grant its operations to a guest or install its Swift/Kotlin handlers.

## Current implementation

| Component                    | Implemented behavior                                                                                                                 | Remaining work                                                 |
| ---------------------------- | ------------------------------------------------------------------------------------------------------------------------------------ | -------------------------------------------------------------- |
| `leptoskit_bridge`           | Bounded binary calls/replies/events; host grants; cancellation; borrowed decoding; asynchronous interpreter calls and event receiver | Native job cancellation adapters                               |
| `leptoskit_wasm`             | Versioned plugin imports; guest memory validation; wake callback                                                                     | App host registration generation                               |
| `leptoskit_bridge_ffi`       | C ABI, Swift ownership wrapper, C++ RAII wrapper, Kotlin/JNI dispatch                                                                | Device integration tests and complete app-host lifecycle       |
| `leptoskit_plugin_crypto`    | SHA-256 and SHA-512 on Rust targets, with direct and bridge APIs                                                                     | Secure random, additional Expo crypto APIs                     |
| `leptoskit_plugin_os`        | Host platform/architecture, direct and bridge APIs                                                                                   | OS version, locale, device details                             |
| `leptoskit_plugin_clipboard` | Text read/write/clear on desktop; Apple and Android handlers; browser read/write; interpreter client                                 | Image/rich-text APIs; device/browser integration verification  |
| CLI                          | `generate app`, `generate plugin`, existing `new`                                                                                    | Native application packaging and automatic plugin registration |

This is an infrastructure increment, **not a completed port of the Expo, Tauri, or Electron catalogs**. The inventory below tracks the remaining scope. Native application scaffolds still inherit the existing Apple host skeleton; they do not yet boot a complete iOS/Android application.

## Host and guest ownership

1. The installed host registers each plugin's method IDs, then explicitly grants operations. These grants are independent of OS permission dialogs.
2. The guest resolves a plugin name once. Calls use numeric IDs and raw bytes.
3. Submission queues work; it never calls UI code while the wasm guest is on the stack. The host drains requests after the guest returns.
4. Swift handlers run on `MainActor`; the Android host must dispatch Kotlin handlers on its UI dispatcher. Slow work suspends or runs on a worker.
5. The host completes replies on the bridge's owning thread and calls `WasmHost::wake_plugins`. The guest executor wakes the waiting futures.
6. Dropping a queued call removes it. Dropping a running call queues a cancellation; drain `Bridge::next_cancelled` (C/Swift: `lk_bridge_next_cancelled`, Kotlin: `NativeBridge.nextCancelled`) after each guest entry point and stop the corresponding native task. Its slot stays occupied until the host takes the notification, so cancellation floods remain bounded. Continue budgeting native resources until cancellation actually finishes. Before unloading a guest, stop subscriptions and cancel jobs; never complete an old call against a replacement bridge.

The core has no global lock or mandatory async runtime. Call slots and queues are preallocated; arithmetic lookup avoids allocating a map entry for each call. An allocator-instrumented test verifies zero heap allocations during 1,000 initialized Rust transport round trips. Rust payload ownership moves through `Bytes`; decoding control messages borrows their fields. The wasm-to-host boundary copies input once because guest linear memory can change after submission. C++ and Swift can retain a Rust buffer with explicit ownership. JNI uses a direct ByteBuffer for requests. Kotlin reply byte arrays may incur a JVM copy in addition to the Rust ownership copy; media should stay in the Rust/C++ path or use a future explicitly owned resource-handle protocol.

C++ handlers can return an existing Rust allocation through `lk_bridge_complete_owned` or `std::move(buffer).complete(bridge, call_id)`. Completion consumes the ownership handle even when the call has been cancelled. Foreign-owned data uses the copying completion API. Tests verify that an owned completion retains the payload address.

The interpreter guest takes one `guest::Events` receiver and routes events by plugin and kind. Its asynchronous `recv()` shares the host wake callback with native-call futures. Dropping a pending wait preserves queued events; stopping native subscriptions remains a plugin operation. `test:plugins:guest` compiles a real Rust wasm guest and checks call cancellation, permission denial, reply wakeups, receiver exclusivity, and event delivery.

This is not a claim of beating Flutter or React Native. The end-to-end path includes the OS operation, thread scheduling, and guest executor. Benchmark those independently from the queue microbenchmark on an otherwise idle machine.

### Resource limits and failure behavior

Defaults: 128 outstanding calls, 256 buffered events, 8 MiB per payload, and 32 MiB total bytes buffered in the bridge. Unread replies and unobserved running-call cancellations retain their call slots. Taking a request transfers byte accounting to the native host, which must budget its running jobs independently. Producers receive an explicit `Busy` error when an event queue is full; a sensor adapter can coalesce readings according to its API instead of silently dropping arbitrary events.

Invalid wasm pointers trap without consuming a reply. Oversized native replies become small terminal error replies. Duplicate and late completions are rejected. IDs are never reused within one bridge. Host grants prevent new calls after revocation; revoking an in-flight native operation requires host cancellation.

The wasm protocol is documented in `crates/leptoskit_wasm/src/plugins.rs`. The native ABI is documented in `crates/leptoskit_bridge_ffi/include/leptoskit_bridge.h`. C pointers are a trusted embedding interface, not a boundary for untrusted guest pointers.

## Generate a project

```sh
leptoskit generate app my-app --platform web
leptoskit generate app my-app-native --platform native
leptoskit generate plugin my-plugin
```

Before the SDK is published, pass `--bridge-path /absolute/path/to/crates/leptoskit_bridge` to the plugin generator. It creates a standalone Cargo workspace with an implemented echo method and an allocation-preservation test. Existing files are not overwritten. Native adapters remain explicit host integrations; the generator does not claim to autolink them yet.

## Research and port inventory

Research snapshot: 20 October 2026. These are upstream feature inventories, not assertions of LeptosKit support. An upstream plugin is not a drop-in Rust library: its lifecycle, permissions, native views, and host registration need to be adapted and tested.

### Flutter as the ecosystem reference

[Flutter's federated plugin model](https://docs.flutter.dev/packages-and-plugins/developing-packages) separates the application API from platform implementations. LeptosKit adopts that separation inside distinct plugin crates, with explicit installed host capabilities. Built-in crates follow the workspace's lockstep releases; generated third-party crates have their own version. [Platform channels](https://docs.flutter.dev/platform-integration/platform-channels) are the reference for asynchronous native calls and events; the LeptosKit transport uses binary payloads and numeric dispatch rather than string method names on each call.

### Expo mobile inventory

Source: [Expo SDK reference](https://docs.expo.dev/versions/latest/) and [Expo Modules API](https://docs.expo.dev/modules/overview/).

The following SDK feature groups still need ports, except the explicitly limited clipboard and crypto implementations above:

| Group                        | SDK features to cover                                                                                                                                                     |
| ---------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Identity and device          | AgeRange, AppIntegrity, AppleAuthentication, Application, Constants, Device, Cellular, Battery, Network, Localization                                                     |
| Sensors                      | Accelerometer, Barometer, DeviceMotion, Gyroscope, Hinge, LightSensor, Magnetometer, Pedometer                                                                            |
| System interaction           | Brightness, Clipboard, Haptics, KeepAwake, Linking, IntentLauncher, LocalAuthentication, ScreenCapture, ScreenOrientation, SecureStore, StoreReview, TrackingTransparency |
| Data and user content        | Asset, Blob, Calendar, Contacts, Crypto, DocumentPicker, FileSystem, ImagePicker, MediaLibrary, SQLite                                                                    |
| Media and native views       | Audio, BlurView, Camera, Checkbox, Font, GlassEffect, GLView, Image, ImageManipulator, LinearGradient, LivePhoto, Maps, MeshGradient, Symbols, Video                      |
| Communication                | AuthSession, Location, MailComposer, Notifications, Print, Sharing, SMS, Speech, WebBrowser                                                                               |
| Host lifecycle               | BackgroundTask, NavigationBar, SplashScreen, StatusBar, SystemUI, TaskManager, Widgets, AppIntents                                                                        |
| Development and distribution | Brownfield, BuildProperties, DevClient, DevMenu, Fingerprint, Manifests, Observe, Server, Updates                                                                         |

Expo's core package, Router, and UI packages are framework facilities rather than individual system plugins. Their relevant navigation and native-view requirements belong to LeptosKit's rendering and tooling work. Deprecated BackgroundFetch/VideoThumbnails and legacy Calendar/Contacts/FileSystem/ MediaLibrary variants need migration guidance rather than a second permanent API. Individual sensors can share an internal implementation while exposing the package boundaries chosen for LeptosKit.

### Tauri desktop and mobile inventory

Source: [official plugin list](https://v2.tauri.app/plugin/), [plugin development](https://v2.tauri.app/develop/plugins/), and [mobile plugins](https://v2.tauri.app/develop/plugins/develop-mobile/).

Remaining ports: Autostart, Barcode Scanner, Biometric, CLI, Deep Linking, Dialog, File System, Geolocation, Global Shortcut, Haptics, HTTP Client, Localhost, Logging, NFC, Notifications, Opener, Persisted Scope, Positioner, Process, Shell, Single Instance, SQL, Store, Stronghold, Updater, Upload, Websocket, and Window State. Clipboard and OS Information have the partial implementations described above.

Tauri's crate plus Swift-package/Android-library structure is useful here. Window-specific plugins require LeptosKit window handles and lifecycle events; they cannot be ported by forwarding calls into a hidden Tauri application. Its official support table also demonstrates why support must be reported per operation and platform.

### Electron desktop baseline

Source: [Electron app and API inventory](https://www.electronjs.org/docs/latest/api/app), [power monitoring](https://www.electronjs.org/docs/latest/api/power-monitor), [utility processes](https://www.electronjs.org/docs/latest/api/utility-process), and [performance guidance](https://www.electronjs.org/docs/latest/tutorial/performance).

Desktop coverage also needs app activation/quit/relaunch, single-instance handoff, file associations, windows and displays, menus, tray, global shortcuts, notifications, native dialogs, clipboard, opening URLs/files, themes, power monitoring and sleep inhibition, secure storage, capture, printing, crash reporting, tracing, worker processes, and signed application updates.

Electron-specific Chromium and Node facilities (webContents/session/protocol, preload scripts, Node integration) are not a reason to embed either runtime. Their relevant capabilities should map to LeptosKit's native renderer, networking, diagnostics, and process APIs. Keep expensive work off the UI thread and verify startup as well as steady-state performance.

### Electron's application tooling

[Electron Forge's build lifecycle](https://www.electronforge.io/core-concepts/build-lifecycle) separates the executable application bundle, distributable installers, and publication. This is a useful model for the remaining LeptosKit desktop tooling:

| Forge responsibility              | LeptosKit integration requirement                                                                                                    |
| --------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------ |
| Package an executable bundle      | Assemble the Rust host, wasm bundle, native plugin libraries, resources, icons, and application identity for one target/architecture |
| Make installers                   | Produce target-specific DMG/Windows/Linux artifacts from that same assembled bundle                                                  |
| Publish artifacts                 | Upload already-built installers and signed update metadata; keep this separate from compiling or invoking a system plugin            |
| Run build hooks                   | Extend the existing `leptoskit_plugin` build hooks, keeping runtime calls in the independent runtime plugin crates                   |
| Build on target operating systems | Use platform CI runners for native SDKs, signing tools, and installed-application smoke tests                                        |

[Forge configuration](https://www.electronforge.io/config/configuration) also handles native-module rebuilding against its bundled runtime. The corresponding LeptosKit requirement is to build Rust, Swift, and Kotlin/C++ artifacts for matching targets and bridge versions, then generate their registration. Cargo dependencies alone do not complete this step. [Forge code signing](https://www.electronforge.io/guides/code-signing) is a reference for making platform signing part of the distribution pipeline.

These are tooling requirements, not implemented packaging commands. The current generator supplies a project layout; complete native bundles, installers, signing, and plugin autolinking remain required before claiming the same ready-to-ship workflow.

## Completion gates for each port

A port needs its own crate, typed Rust API, actual native/browser implementations, capability declarations, permission-denial/cancellation/lifecycle tests, and device or browser verification. OS-specific facilities such as global desktop shortcuts or app-store review prompts must expose explicit unavailability where the platform has no corresponding API. Never report a no-op as successful support.

Next implementation layers: finish generated mobile hosts and registration; port common storage, permissions, device, haptics, network, and lifecycle APIs; then media/sensors/background services and desktop window integration. The catalog remains open until every listed capability is implemented or has a documented platform-specific exclusion.
