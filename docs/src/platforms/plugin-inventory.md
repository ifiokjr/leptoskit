# System plugin port inventory

Research snapshot: **20 October 2026**. This is a scope and acceptance ledger, not a platform-support announcement. The upstream inventories change; recheck their linked reference pages when beginning a port.

The API families below come from the [Expo SDK reference](https://docs.expo.dev/versions/latest/) and [Tauri official plugin catalog](https://v2.tauri.app/plugin/). The crate boundaries, priorities, and acceptance requirements are LeptosKit design proposals. They do not imply upstream compatibility. “Port” means providing the useful system capability through idiomatic Rust APIs and the LeptosKit host, without running a hidden React Native or Tauri application.

## Status vocabulary

| Status            | Meaning                                                                                                   |
| ----------------- | --------------------------------------------------------------------------------------------------------- |
| Partial           | Source implements the explicitly named subset. Other methods and platforms remain open.                   |
| In progress       | Active implementation work; no release or platform coverage is asserted here.                             |
| Planned           | No completed runtime plugin is recorded in this inventory.                                                |
| Framework/tooling | Belongs to rendering, application lifecycle, or build tooling rather than an independent runtime service. |
| Migration only    | Document the replacement; avoid preserving a deprecated API indefinitely.                                 |

Each runtime plugin should remain its own crate. Platform implementations may live in that crate as Rust modules, a Swift package, and Kotlin sources. An internal shared codec or native helper does not turn unrelated plugins into a single omnibus dependency.

## Current plugin support matrix

All crate names below have the prefix `leptoskit_plugin_`. Each row describes a **partial** capability port, including current unmerged work. Platform cells describe implemented code paths, not a promise that every device, permission policy or packaged installation has been verified. The crate READMEs and Cargo runtime metadata define the operation-level contracts.

**Auto** means a selected direct dependency has generated native registration metadata. **Direct** means the host/browser calls its Rust API; this does not imply interpreter guest access. **Manual** means the host must construct and register an explicitly configured handler. **Unavailable** means the capability is absent, even if a stub handler compiles.

### Identity, system state and interaction

| Crate          | Implemented subset                                                                              | iOS / Android                                    | macOS                                                    | Windows / Linux                                         | Browser                                                        | Remaining scope                                                                                           |
| -------------- | ----------------------------------------------------------------------------------------------- | ------------------------------------------------ | -------------------------------------------------------- | ------------------------------------------------------- | -------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------- |
| `os`           | Host compilation platform and architecture                                                      | Auto Rust                                        | Auto Rust                                                | Direct/manual Rust                                      | Direct reports `web`/`wasm32`                                  | OS release/version and broader OS information                                                             |
| `crypto`       | SHA-256 and SHA-512 over bytes                                                                  | Auto Rust                                        | Auto Rust                                                | Direct/manual Rust                                      | Direct Rust                                                    | Secure randomness, UUIDs and other crypto operations                                                      |
| `application`  | App ID, display name, release version and build number                                          | Auto native package metadata                     | Auto bundle metadata                                     | Explicit host-provided snapshot                         | Explicit app-provided snapshot only                            | Install identifiers/dates, referrer and store environment                                                 |
| `device`       | OS/version, architecture, memory and platform-dependent manufacturer/model/kind/simulator hints | Auto; unavailable fields remain unknown          | Auto Swift; direct Rust subset                           | Direct/manual Rust OS/architecture/RAM subset           | Low-entropy platform hint only                                 | Root/sideload/heap/CPU feature APIs; no stable device ID                                                  |
| `localization` | Preferred locales, calendar/timezone, weekday and clock preference where exposed                | Auto                                             | Auto Foundation; direct Rust subset                      | Direct/manual locales/timezone only                     | Direct Navigator/Intl subset                                   | Change events, direction, separators, currencies and measurement preferences                              |
| `clipboard`    | Read/write/clear text                                                                           | Auto                                             | Auto Swift; direct Rust                                  | Direct/manual Rust                                      | Direct Clipboard API; clear writes empty text                  | Images, rich content and listeners                                                                        |
| `haptics`      | Selection, impact and notification feedback; Android-specific constants                         | Auto                                             | Auto trackpad patterns                                   | Unavailable                                             | Direct vibration adaptation                                    | Physical feedback depends on hardware/policy; no equivalence between effects                              |
| `battery`      | Charge/state/power-saving snapshot, wait for change, Android optimization-policy query          | Auto; one change waiter                          | Auto IOKit notifications                                 | Direct enumeration/polling; manual bridge snapshot only | Direct Battery Status API snapshot/wait when available         | Desktop/browser power-saving policy unknown; desktop polling is not native notifications                  |
| `network`      | Connectivity hints and demand-driven latest-state changes                                       | Auto; Android additionally exposes OS validation | Auto NWPathMonitor                                       | Direct interfaces; manual bridge snapshot only          | Direct online/NetworkInformation hints and waits               | Airplane mode, SSID/MAC, local-IP RPC; connectivity does not prove server reachability                    |
| `linking`      | Validated outbound HTTP(S), mail, telephone and SMS links                                       | Auto open/can-open/app settings                  | Auto open/can-open; direct opener                        | Direct/manual open only                                 | Direct user-gesture `window.open`                              | Inbound cold/warm links, associations, arbitrary custom schemes and opening/revealing files               |
| `permissions`  | Camera/microphone status and request                                                            | Auto; Activity broker on Android                 | Auto AVFoundation; purpose strings/entitlements required | Unavailable                                             | Direct Permissions API and temporary getUserMedia with cleanup | Other permission families, media capture and uniform permanent-denial detection                           |
| `brightness`   | Get/set temporary brightness and restore original setting                                       | Auto; iOS screen, Android activity window        | Unavailable                                              | Unavailable                                             | Unavailable                                                    | Persistent settings, adaptive mode and brightness change events                                           |
| `keep_awake`   | Owned display-idle inhibition until cancellation/drop                                           | Auto pending lease; background cleanup           | Auto Swift; direct Rust lease                            | Direct Rust lease                                       | Direct Screen Wake Lock holding future                         | Background execution is not granted; OS/browser policy may revoke a lease                                 |
| `speech`       | Voice discovery, speak-until-finished, stop and owned-active status                             | Auto                                             | Auto AVSpeechSynthesizer                                 | Unavailable                                             | Direct speechSynthesis                                         | Pause/resume, word boundaries, audio-session configuration; engine completion is not audible-output proof |

### Storage and networking

| Crate          | Implemented subset                                                                            | iOS / Android                          | macOS                                     | Windows / Linux                            | Browser                                           | Remaining scope                                                                                  |
| -------------- | --------------------------------------------------------------------------------------------- | -------------------------------------- | ----------------------------------------- | ------------------------------------------ | ------------------------------------------------- | ------------------------------------------------------------------------------------------------ |
| `secure_store` | Get/set/delete binary secrets up to 2 KiB                                                     | Auto Keychain / Keystore AES-GCM files | Auto Swift Keychain; manual Rust Keychain | Manual Credential Manager / Secret Service | Unavailable                                       | Biometrics, access groups, configurable accessibility, enumeration; not Stronghold               |
| `filesystem`   | Private read/write/list/mkdir/remove/stat/rename; bounded files and directory capabilities    | Auto Rust worker                       | Auto Rust worker                          | Direct/manual Rust                         | Direct OPFS; file rename requires browser support | User-selected/external files, watchers, downloads, recursive deletion and large-file streaming   |
| `store`        | JSON settings, staged edits, explicit save/load, atomic ordered batches                       | Auto Rust worker                       | Auto Rust worker                          | Direct/manual Rust                         | Direct localStorage                               | Cross-process isolation, autosave/change streams; not encrypted                                  |
| `sqlite`       | Parameterized writes, bounded read-only queries and atomic transaction batches                | Auto bundled SQLite worker             | Auto bundled SQLite worker                | Direct/manual bundled SQLite               | Types/codecs only; no backend                     | Browser SQLite/OPFS worker, FTS/virtual tables, encryption, import/export and subscriptions      |
| `http`         | Allowed-origin async requests, bounded binary bodies/headers, deadlines and drop cancellation | Direct Rust with Tokio                 | Direct Rust with Tokio                    | Direct Rust with Tokio                     | Direct fetch; CORS still applies                  | Guest adapter, streaming-to-caller API, redirects and upload/progress APIs                       |
| `websocket`    | Allowed-origin async sockets, bounded messages/queues, backpressure and close deadlines       | Direct Rust with Tokio                 | Direct Rust with Tokio                    | Direct Rust with Tokio                     | Direct WebSocket                                  | Guest handles, reconnect policy, browser custom headers and TLS/client-certificate customization |

The three storage workers defer disk I/O until dispatch. Cancellation discards their reply but cannot promise rollback of an already-started operation. Private files are limited to 1 MiB per operation; SQLite requests/results are also bounded to 1 MiB and settings documents to 512 KiB. These are operation bounds, not a total disk quota. Secure storage never substitutes plaintext files or browser storage for an unavailable credential service.

### Sensors and desktop host services

| Crate             | Implemented subset                                                       | iOS / Android                 | macOS                            | Windows / Linux                                            | Browser                                 | Remaining scope                                                                  |
| ----------------- | ------------------------------------------------------------------------ | ----------------------------- | -------------------------------- | ---------------------------------------------------------- | --------------------------------------- | -------------------------------------------------------------------------------- |
| `accelerometer`   | Availability and one fresh timestamped sample including gravity, in m/s² | Auto CoreMotion/SensorManager | Unavailable                      | Unavailable                                                | Direct Generic Sensor/DeviceMotion      | Lossless recording, batching and physical-device coverage                        |
| `gyroscope`       | Availability and one fresh angular-velocity sample, in rad/s             | Auto CoreMotion/SensorManager | Unavailable                      | Unavailable                                                | Direct Generic Sensor/DeviceMotion      | Fusion/orientation and lossless recording                                        |
| `magnetometer`    | Availability and one fresh field sample, in µT                           | Auto CoreMotion/SensorManager | Unavailable                      | Unavailable                                                | Direct Generic Sensor when available    | Explicit calibrated/uncalibrated modes and compass heading                       |
| `barometer`       | Availability and one fresh pressure sample, in hPa                       | Auto CoreMotion/SensorManager | Unavailable                      | Unavailable                                                | Unavailable                             | Relative altitude and continuous recording                                       |
| `dialog`          | Async file/directory/save/message dialogs                                | Unavailable                   | Direct parent-window API         | Direct parent-window API; Linux portal/Zenity requirements | Direct bounded open-file selection only | Guest adapter; mobile document providers; browser save/directory/message dialogs |
| `autostart`       | Enable/disable/query exact host-owned startup entry                      | Unavailable                   | Manual LaunchAgent configuration | Manual Windows Run / XDG configuration                     | Unavailable                             | Packaged login verification, OS startup-policy state; no automatic constructor   |
| `single_instance` | Process-lifetime OS lock with retained guard                             | Unavailable                   | Direct host startup guard        | Direct host startup guard                                  | Unavailable                             | Second-launch arguments, activation/focus handoff; no guest methods              |

The four sensor crates share `leptoskit_sensor` acquisition/codecs without merging their capability boundaries. Each sample request stops acquisition on completion, cancellation, close or native backgrounding; one request per sensor may wait at a time. Browser permissions and iOS purpose strings remain necessary. Simulators and API availability checks do not establish physical sensor accuracy.

### Infrastructure and distribution

| Component                                   | Implemented subset                                                                                                                                          | Remaining scope                                                                                           |
| ------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------- |
| `leptoskit_bridge` / `leptoskit_bridge_ffi` | Bounded binary transport, grants, cancellation, retained C/Swift/C++/JNI buffers, Rust worker execution and native job accounting through actual completion | Plugin-specific resource budgets, broader async guest resource protocols and end-to-end device benchmarks |
| `leptoskit_cli`                             | iOS/Android/macOS host generation/builds, editable host files and metadata-driven registration of selected direct dependencies                              | Windows/Linux scaffolds, richer release configuration, installers/signing/notarization/publication        |
| `leptoskit_codepush`                        | Signed Wasm manifests, verification, staging and delta machinery                                                                                            | Installed-host activation/recovery/rollback and native executable updates                                 |
| `leptoskit_plugin`                          | Build-time hooks                                                                                                                                            | Keep build plugins distinct from runtime capability crates                                                |

The [runtime contracts](plugins.md) explain ownership, threading, native work limits and the narrow performance evidence. The [native host project guide](host-projects.md) describes generated projects. A working Rust backend on Windows/Linux does not mean those application scaffolds exist.

## Expo inventory

The tables cover every entry under **Expo SDK** in the reference sidebar at the snapshot date. Router and Expo UI are classified separately below. SDK feature names do not always correspond one-to-one to npm packages: the individual sensors, for example, are surfaced through the sensors package. Proposed Rust crate separation follows capability boundaries.

### System, identity and data

| Expo entry                      | Rust destination                           | Status / acceptance scope                                                                                |
| ------------------------------- | ------------------------------------------ | -------------------------------------------------------------------------------------------------------- |
| AgeRange                        | `age_range`                                | Planned; explicit unavailable/declined states and platform-specific age-range semantics                  |
| AppIntegrity                    | `app_integrity`                            | Planned; platform attestation with a server-verification contract                                        |
| AppleAuthentication             | `apple_authentication`                     | Planned; native sign-in, cancellation and credential-state changes                                       |
| Application                     | `application`                              | Partial; installed app ID/name/version/build; installation/referrer/store APIs remain                    |
| Asset                           | `asset`                                    | Planned; packaged resources, download/cache ownership and content verification                           |
| Battery                         | `battery`                                  | Partial; snapshot, coalesced change wait and Android optimization-policy query                           |
| Blob                            | `blob` or an owned-resource facility       | Planned; avoid repeatedly copying large bytes through control messages                                   |
| Brightness                      | `brightness`                               | Partial; mobile temporary override/restoration; persistent system settings and events remain             |
| Calendar; Calendar (legacy)     | `calendar`                                 | Planned; current API first, legacy migration guidance                                                    |
| Cellular                        | `cellular`                                 | Planned; supported carrier/radio data with unknown and restricted fields represented explicitly          |
| Clipboard                       | `clipboard`                                | Partial; text operations only                                                                            |
| Constants                       | application configuration/runtime metadata | Framework/tooling; separate build facts from live device facts                                           |
| Contacts; Contacts (legacy)     | `contacts`                                 | Planned; current API first, legacy migration guidance                                                    |
| Crypto                          | `crypto`                                   | Partial; two digest algorithms only                                                                      |
| Device                          | `device`                                   | Partial; basic information with explicit unknowns; root/sideload/heap/CPU feature APIs remain            |
| DocumentPicker                  | `document_picker`                          | Planned; cancellation, security-scoped access and returned-resource lifetime                             |
| FileSystem; FileSystem (legacy) | `filesystem`                               | Partial; private bounded files; external providers, downloads and streaming remain                       |
| Haptics                         | `haptics`                                  | Partial; portable effects and Android-specific constants; hardware behavior varies                       |
| IntentLauncher                  | `intent_launcher`                          | Planned; Android-specific intents with validated arguments                                               |
| KeepAwake                       | `keep_awake`                               | Partial; owned display wake leases with cancellation/background cleanup                                  |
| Linking                         | `linking`                                  | Partial; outbound safe schemes/app settings; inbound launch routing and associations remain              |
| LocalAuthentication             | `local_authentication`                     | Planned; challenge results, cancellation, enrolled/available distinctions                                |
| Localization                    | `localization`                             | Partial; platform-dependent locale/calendar/timezone snapshots; events and formatting preferences remain |
| Network                         | `network`                                  | Partial; state hints and coalesced changes; airplane mode and local-IP RPC remain                        |
| ScreenCapture                   | `screen_capture`                           | Planned; distinguish capture prevention, capture detection and taking screenshots                        |
| ScreenOrientation               | `screen_orientation`                       | Planned; lock leases, reported orientation and change events                                             |
| SecureStore                     | `secure_store`                             | Partial; small-secret get/set/delete; biometric/access-group/accessibility options remain                |
| SQLite                          | `sqlite`                                   | Partial; native parameterized reads/writes/transactions; browser backend and extended APIs remain        |
| StoreReview                     | `store_review`                             | Planned; availability and request acceptance, without promising a prompt                                 |
| TrackingTransparency            | `tracking_transparency`                    | Planned; native status/request flow and unsupported-platform result                                      |

The [Battery API](https://docs.expo.dev/versions/latest/sdk/battery/) informs snapshots, change observation and Android optimization queries; LeptosKit's repeated waits coalesce intermediate states. The [Device API](https://docs.expo.dev/versions/latest/sdk/device/) is broader than the implemented information snapshot. [Haptics](https://docs.expo.dev/versions/latest/sdk/haptics/) has platform-dependent behavior; desktop trackpad feedback and browser vibration are adaptations, not identical hardware effects. The separate `permissions` crate currently supplies camera/microphone status and request flows, not every permission needed by this inventory.

### Sensors

| Expo entry    | Proposed crate             | Required behavior                                                                  |
| ------------- | -------------------------- | ---------------------------------------------------------------------------------- |
| Accelerometer | `accelerometer`            | Partial; timestamped SI readings and requested interval; one fresh sample per call |
| Barometer     | `barometer`                | Partial; timestamped hPa readings and hardware availability; no altitude estimate  |
| DeviceMotion  | `device_motion`            | Planned; coordinate frames and sensor-fusion semantics                             |
| Gyroscope     | `gyroscope`                | Partial; timestamped rad/s readings and requested interval                         |
| Hinge         | `hinge`                    | Planned; foldable hardware availability and posture/angle semantics                |
| LightSensor   | `light_sensor`             | Planned; illuminance and unavailable hardware                                      |
| Magnetometer  | `magnetometer`             | Partial; timestamped µT readings; explicit calibration modes remain                |
| Pedometer     | `pedometer`                | Planned; permissions, query interval and live updates                              |
| Sensors       | `leptoskit_sensor` helpers | Partial; shared native/browser acquisition lifecycle for four separate plugins     |

Every continuous sensor needs bounded delivery, documented coalescing, unsubscribe-on-drop, and background/foreground behavior. An event transport alone does not implement a sensor plugin.

### Communication, media and user interaction

| Expo entry                          | Rust destination                       | Status / acceptance scope                                                                                |
| ----------------------------------- | -------------------------------------- | -------------------------------------------------------------------------------------------------------- |
| Audio (expo-audio)                  | `audio`                                | Planned; capture/playback sessions, interruption and resource lifetime                                   |
| AuthSession                         | `auth_session`                         | Planned; authentication browser session, redirect validation and cancellation                            |
| Camera                              | `camera`                               | Planned; native preview integration plus capture/permission lifecycle                                    |
| Font                                | font/rendering integration             | Framework/tooling; asset loading, registration and text-layout invalidation                              |
| GLView                              | graphics surface integration           | Framework/tooling; explicit graphics context and surface ownership                                       |
| Image                               | image/rendering integration            | Framework/tooling; decode/cache/resource lifecycle                                                       |
| ImageManipulator                    | `image_manipulator`                    | Planned; owned image resources and off-UI-thread transforms                                              |
| ImagePicker                         | `image_picker`                         | Planned; selection, cancellation and temporary resource ownership                                        |
| LivePhoto                           | `live_photo` plus renderer integration | Planned; platform-specific media view and playback lifecycle                                             |
| Location; Location (next)           | `location`                             | Planned; select a stable contract, then map newer APIs without duplicate permanent clients               |
| MailComposer                        | `mail_composer`                        | Planned; availability, attachments and presentation completion                                           |
| Maps                                | `maps` plus renderer integration       | Planned; native view handles and map event ownership                                                     |
| MediaLibrary; MediaLibrary (legacy) | `media_library`                        | Planned; limited-library grants, asset changes and migration guidance                                    |
| Notifications                       | `notifications`                        | Planned; local scheduling, channels/categories, actions and remote registration as separate capabilities |
| Print                               | `print`                                | Planned; document ownership, print dialog cancellation and platform output                               |
| Sharing                             | `sharing`                              | Planned; user-selected destinations and resource grants                                                  |
| SMS                                 | `sms`                                  | Planned; composer presentation and availability                                                          |
| Speech                              | `speech`                               | Partial; voices, speak, stop and active status; pause/resume and word events remain                      |
| Video (expo-video)                  | `video`                                | Planned; native surface, playback state, audio focus and teardown                                        |
| VideoThumbnails                     | video frame extraction                 | Migration only; deprecated upstream entry, no separate legacy API by default                             |
| WebBrowser                          | `web_browser`                          | Planned; external/in-app browser ownership distinct from authentication sessions                         |

### Lifecycle, platform UI and developer tooling

| Expo entry                                                             | LeptosKit destination                       | Status / acceptance scope                                                                    |
| ---------------------------------------------------------------------- | ------------------------------------------- | -------------------------------------------------------------------------------------------- |
| Expo                                                                   | core runtime integration                    | Framework/tooling; no compatibility facade for the JavaScript framework                      |
| AppIntents                                                             | `app_intents` and host extension generation | Planned; alpha upstream entry; isolate unstable platform configuration                       |
| BackgroundFetch                                                        | background task migration                   | Migration only; deprecated upstream entry                                                    |
| BackgroundTask                                                         | `background_task`                           | Planned; OS scheduling, expiration and completion, without guarantees about execution time   |
| BlurView; Checkbox; GlassEffect; LinearGradient; MeshGradient; Symbols | renderer/components                         | Framework/tooling; native visual components, not system-service ports                        |
| Brownfield                                                             | host embedding APIs                         | Framework/tooling; embed LeptosKit in an existing application                                |
| BuildProperties                                                        | CLI/native build configuration              | Framework/tooling; typed target settings and reproducible generated projects                 |
| DevClient; DevMenu                                                     | developer host and diagnostics              | Framework/tooling; inspect/reload/debug workflow                                             |
| Fingerprint                                                            | build compatibility fingerprint             | Framework/tooling; include native plugin graph and bridge version                            |
| Manifests                                                              | application/update metadata                 | Framework/tooling; reuse versioned config and signed bundle metadata where applicable        |
| NavigationBar; StatusBar; SystemUI                                     | native host presentation                    | Framework/tooling; implement platform UI state with restoration and scene/activity ownership |
| Observe                                                                | `diagnostics`                               | Planned; startup/interaction spans and errors; backend integration is a separate choice      |
| Server                                                                 | server/build tooling                        | Framework/tooling; route/runtime integration rather than a device permission                 |
| SplashScreen                                                           | host startup lifecycle                      | Framework/tooling; first-frame readiness and failure recovery                                |
| TaskManager                                                            | background task runtime                     | Framework/tooling; registration, restart and headless task lifecycle                         |
| Updates                                                                | `leptoskit_codepush` plus host integration  | Partial; verified staging exists, installed application activation/recovery remains          |
| Widgets                                                                | `widgets` and extension generation          | Planned; separate extension targets, shared data and refresh lifecycle                       |

[Observe](https://docs.expo.dev/versions/latest/sdk/observe/) includes performance measurements and application events; a metrics API should not require a particular hosted backend. [Server](https://docs.expo.dev/versions/latest/sdk/server/) belongs with server/build integration. [AppIntegrity](https://docs.expo.dev/versions/latest/sdk/app-integrity/) requires platform attestation semantics rather than a local “trusted device” boolean.

Expo Router (including its navigation/UI entries) and Expo UI are renderer/routing work. The separate third-party-library section in Expo's reference is not an Expo-built-in plugin inventory; it should be evaluated separately if requested. This distinction does not exclude native view work such as camera, maps or media from the project.

## Tauri official plugins

All **30** official entries in the [Tauri catalog](https://v2.tauri.app/plugin/) are mapped below. Community plugins are outside this finite inventory. Tauri's own platform support varies; a row here is not a promise that every OS exposes the same operation.

| Tauri entry                  | LeptosKit destination                 | Status / next acceptance requirement                                                                              |
| ---------------------------- | ------------------------------------- | ----------------------------------------------------------------------------------------------------------------- |
| Autostart                    | `autostart`                           | Partial; explicit desktop configuration and exact-entry enable/disable/query; packaged login verification remains |
| Barcode Scanner              | `barcode_scanner`                     | Planned; camera permission, decoding and scanner lifecycle                                                        |
| Biometric                    | `local_authentication`                | Planned; share the authentication capability rather than duplicate it                                             |
| Command Line Interface (CLI) | `arguments`                           | Planned; application arguments, separate from the development CLI                                                 |
| Clipboard                    | `clipboard`                           | Partial; expand beyond text only as explicit operations                                                           |
| Deep Linking                 | `linking`                             | Planned inbound capability; current linking crate only handles outbound links/settings                            |
| Dialog                       | `dialog`                              | Partial direct desktop prompts/browser file selection; guest and mobile presentation adapters remain              |
| File System                  | `filesystem`                          | Partial; bounded app-private file operations; user grants/watchers/streaming remain                               |
| Geolocation                  | `location`                            | Planned; same portable location contract as the mobile port                                                       |
| Global Shortcut              | `global_shortcut`                     | Planned; registration conflict, release on teardown and platform restrictions                                     |
| Haptics                      | `haptics`                             | Partial; portable effects and Android constants with platform-specific availability                               |
| HTTP Client                  | `http`                                | Partial direct Rust/browser bounded requests; guest adapter and streamed output remain                            |
| Localhost                    | host-serving integration              | Planned; only if an application needs a loopback server; bind/authentication/lifetime contract                    |
| Logging                      | `logging`                             | Planned; structured records, rotation and shutdown flush                                                          |
| NFC                          | `nfc`                                 | Planned; session ownership, permissions and hardware availability                                                 |
| Notifications                | `notifications`                       | Planned; share notification types with mobile while exposing OS differences                                       |
| Opener                       | `linking` plus file-opening operation | Partial outbound URLs; reveal/open-file behavior remains                                                          |
| OS Information               | `os`                                  | Partial; platform/architecture only                                                                               |
| Persisted Scope              | capability persistence                | Planned; explicit grants, expiration/revocation and app identity                                                  |
| Positioner                   | `window` placement operations         | Planned; native window/display handles and work-area changes                                                      |
| Process                      | `application` lifecycle               | Planned; exit/relaunch with orderly resource shutdown                                                             |
| Shell                        | `shell`                               | Planned; permitted executable/arguments, streams, cancellation and child ownership                                |
| Single Instance              | `single_instance`                     | Partial host-owned process lock; second-launch argument/activation handoff remains                                |
| SQL                          | `sqlite`                              | Partial bounded SQLite reads/writes/transaction batches; migrations API and other database drivers remain         |
| Store                        | `store`                               | Partial staged JSON settings, save/load and atomic batches; autosave/change events remain                         |
| Stronghold                   | `stronghold`                          | Planned; encrypted vault/session APIs, distinct from OS secure storage                                            |
| Updater                      | `updater`                             | Planned; native binary installation/rollback distinct from Wasm bundle updates                                    |
| Upload                       | `transfer`                            | Planned; bounded streaming, progress and cancellation over shared HTTP machinery                                  |
| Websocket                    | `websocket`                           | Partial direct Rust/browser sockets, bounded queues and close handshake; guest handles remain                     |
| Window State                 | `window_state`                        | Planned; restore valid bounds after display changes, persist at defined lifecycle points                          |

[SQL](https://v2.tauri.app/plugin/sql/) supports more than a SQLite-only contract. [Stronghold](https://v2.tauri.app/plugin/stronghold/) is a vault capability, so an OS keychain wrapper must not be described as its complete port.

## Electron desktop baseline

Electron is useful here as an inventory of application responsibilities. Its Chromium/Node architecture does not need to be reproduced to supply those responsibilities.

| Electron reference                                                           | LeptosKit requirement                                                             | Current disposition                                                                                            |
| ---------------------------------------------------------------------------- | --------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------- |
| [app](https://www.electronjs.org/docs/latest/api/app)                        | Activation/quit/relaunch, app paths, file/URL associations, second-launch handoff | macOS host/app directories and single-instance lock exist; portable lifecycle, associations and handoff remain |
| [BrowserWindow](https://www.electronjs.org/docs/latest/api/browser-window)   | Create/close/focus windows, bounds, fullscreen and per-window lifecycle           | Requires native window handles and events before window plugins can work                                       |
| [Menu](https://www.electronjs.org/docs/latest/api/menu)                      | Application/context menus, standard command roles, accelerators and enabled state | Planned `menu` crate                                                                                           |
| [Tray](https://www.electronjs.org/docs/latest/api/tray)                      | Owned tray icon, menu/actions, click events and teardown                          | Planned `tray` crate                                                                                           |
| [dialog](https://www.electronjs.org/docs/latest/api/dialog)                  | Open/save/message dialogs and cancellation                                        | Direct desktop `dialog` APIs; guest/window lifecycle integration remains                                       |
| [Notification](https://www.electronjs.org/docs/latest/api/notification)      | Native notifications and interaction callbacks                                    | Planned shared `notifications` crate                                                                           |
| [globalShortcut](https://www.electronjs.org/docs/latest/api/global-shortcut) | Shortcuts while the app is unfocused                                              | Planned `global_shortcut` crate                                                                                |
| [powerMonitor](https://www.electronjs.org/docs/latest/api/power-monitor)     | Suspend/resume, idle and power state events                                       | Battery observation and keep-awake exist; suspend/resume/idle APIs remain                                      |
| [safeStorage](https://www.electronjs.org/docs/latest/api/safe-storage)       | OS-protected secret storage with actual protection availability reported          | Partial `secure_store` credential entries; not a general encrypt/decrypt facade                                |
| [autoUpdater](https://www.electronjs.org/docs/latest/api/auto-updater)       | Installed native application updates                                              | Planned `updater`; separate from guest bundle staging                                                          |

The broader [Electron API index](https://www.electronjs.org/docs/latest/api/app) also motivates display enumeration, system theme changes, sleep inhibition, desktop capture, printing, sharing, crash reporting, tracing, application purchasing, push registration and owned worker/utility processes. Track these as `display`, `theme`, `keep_awake`, `screen_capture`, `print`, `sharing`, `diagnostics`, `purchases`, `notifications` and process capabilities as their contracts are designed.

Chromium-specific `webContents`, browser sessions, preload scripts, DOM frames and Node integration are not required for a native renderer. Their underlying user needs—network requests, permissions, external content and isolated work—need explicit LeptosKit APIs instead. Touch Bar and other narrowly available UI integrations should have honest platform-specific contracts.

### Desktop application tooling

[Electron Forge's build lifecycle](https://www.electronforge.io/core-concepts/build-lifecycle) separates packaging, making distributables and publishing. Apply that separation to LeptosKit rather than hiding signing, network publication or installer mutation inside a runtime plugin.

| Work item            | Required result                                                                                           | Existing starting point                                                           |
| -------------------- | --------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------- |
| Generate desktop app | Editable host, window lifecycle, assets and plugin registration for macOS/Windows/Linux                   | macOS AppKit host generated; Windows/Linux scaffolds remain                       |
| Compile native graph | Matching Rust/Swift/Kotlin/C++ target artifacts and bridge versions; deterministic registration           | iOS/Android/macOS metadata discovery and build pipeline                           |
| Package application  | Executable, guest bundle, native libraries, resources, icons and app identity in a runnable target layout | iOS/Android host assembly and runnable macOS `.app`; Windows/Linux layouts remain |
| Make distributables  | OS-specific installer/archive outputs from the packaged application                                       | No installer parity claimed                                                       |
| Sign and notarize    | Platform signing with explicit credentials/provisioning and verifiable output                             | Debug/unsigned mobile builds are not release distribution                         |
| Publish              | Upload already-built artifacts and signed update metadata                                                 | Keep separate from compilation and local plugin calls                             |
| Update               | Verify, stage, activate, recover and roll back with pinned trust                                          | Existing signed Wasm bundle machinery covers only part of this                    |
| Diagnose             | Versioned crash/symbol artifacts, traces and startup metrics                                              | Add platform symbol collection and host instrumentation                           |
| Verify installation  | Launch installed artifact, exercise plugin operation, upgrade and uninstall                               | Extend generated app UI tests into packaged-application tests                     |

[Electron's native module documentation](https://www.electronjs.org/docs/latest/tutorial/using-native-node-modules) illustrates the importance of matching native artifacts to their runtime. LeptosKit's equivalent is matching CPU/OS target, native SDK linkage and the C/guest ABI—not adding Node module rebuilding to this project.

## Implementation order and completion gates

This ordering is a proposal based on dependencies and common application needs, not an upstream ranking.

1. **Finish guest integration for direct APIs:** an asynchronous, cancellation-aware adapter for HTTP/WebSocket and live-parent-window dialog presentation. Keep explicit destination scopes and bounded resource ownership; do not block the interpreter waiting for I/O.
2. **Verify and extend existing ports:** generated-app permission/lifecycle flows, physical haptics/sensors/brightness/speech, platform-specific credential failures, installed startup behavior and browser policy. Existing tests do not imply physical-device or release-package parity.
3. **Add independent common services:** orientation, local authentication, sharing, document picker and notifications. Notifications need local scheduling/actions and remote-token registration as separately verified operations. Inbound links need launch routing and generated associations.
4. **Complete desktop application ownership:** Windows/Linux scaffolds, window/display APIs, menus, tray, shortcuts, second-launch handoff and native updater. Build installers/signing/notarization alongside these so installed-app behavior can be tested.
5. **Expand acquisition and media:** remaining sensors (motion fusion, hinge, light, pedometer), location, camera, barcode, audio/video, images, contacts, calendar, media library and NFC. Large data needs bounded streaming or owned resources; native views require renderer integration.
6. **Add background and extension integration:** task execution, notification/background launch, widgets, app intents, purchases, attestation and platform-specific identity features.

A port is complete only when its public operation list, target matrix and failure behavior are documented and backed by tests. For each operation record:

- Rust API and independent crate; explicit wire version and checked decoding.
- Real native/browser implementation, supported OS versions and hardware requirements.
- Required build contributions: permissions, usage strings, entitlements, URL schemes, extensions or dependencies.
- Permission denied, user cancellation, unavailable capability and native failure as distinct outcomes where the platform distinguishes them.
- App close, guest replacement, backgrounding, pending work cancellation and subscription cleanup.
- Bounded memory and work queues; blocking I/O off the UI thread; payload/FFI ownership tests.
- Device/browser evidence for user-facing behavior, with simulator limits stated.

Do not mark an entire upstream module complete after adding one method. Do not claim that a bridge microbenchmark proves an end-to-end advantage over Flutter or React Native. Measure warmed and cold calls, event throughput, allocations, large payloads, UI-thread time and platform-operation latency separately.
