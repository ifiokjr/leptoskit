# Runtime plugins

Runtime plugins give an installed LeptosKit host access to operating-system services. They are separate from `leptoskit_plugin`, whose hooks customize builds. Each runtime plugin is an independent Cargo crate. Adding a dependency does not grant its operations to a guest or install its Swift/Kotlin handlers.

## Current implementation

### Image transforms and capture policy

`leptoskit_plugin_image_manipulator` supplies ordered JPEG/PNG crop, resize, rotation, flip and encoding operations. Select `image_manipulator` for automatic native Rust-worker registration, or call the engine directly in a browser. Requests and results are bounded to 4 MiB, decoded images to four million pixels and 8,192 pixels per dimension, and each request to 16 operations. Dimension checks precede pixel decoding. These checks and codec allocation limits reduce resource use; they do not impose a hard process-memory limit on every decoder-private allocation. The crate requires Rust 1.88.

`leptoskit_plugin_screen_capture` registers as `screen-capture`. Android provides cancellation-owned `FLAG_SECURE` leases and, on Android 14 or later, Activity screenshot notifications. iOS provides owned-scene recording-state observation and app-wide screenshot notifications. iOS capture prevention and desktop/browser backends are unavailable. Each request reports its actual scope; screenshot notification cannot undo a capture. This plugin never captures or reads pixels.

Capture observers exist only while a request is pending and are removed on cancellation, backgrounding or close. Android prevention leases preserve the original secure flag and remain active while the Activity is stopped, until release or destruction. Other code changing that same flag must coordinate with the lease. Synthetic lifecycle tests do not establish compositor enforcement or physical-device recording behavior.

The [plugin inventory](plugin-inventory.md) records every current plugin crate, its implemented operations, platform coverage, registration path, and remaining Expo/Tauri/Electron scope. These are focused ports; no complete upstream API compatibility is claimed.

| Component                | Implemented behavior                                                                                                                                                                                                                                                                                                          | Remaining work                                                                |
| ------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------- |
| `leptoskit_bridge`       | Bounded binary calls/replies/events, host grants, cancellation, borrowed decoding, asynchronous interpreter calls and event receiver                                                                                                                                                                                          | Per-plugin contracts and performance validation remain necessary              |
| `leptoskit_wasm`         | Versioned plugin imports, guest memory validation, wake callback and guest executor polling                                                                                                                                                                                                                                   | Live guest replacement                                                        |
| `leptoskit_bridge_ffi`   | C ABI, Swift and C++ ownership wrappers, Kotlin/JNI dispatch, retained Rust workers, native application lifecycle and controls                                                                                                                                                                                                | Broader widget coverage and rendering benchmarks                              |
| Native hosts             | Generated iOS, Android and macOS apps, selected-plugin registration, app directory context and cancellation-aware job accounting                                                                                                                                                                                              | Windows/Linux application scaffolds and complete release packaging            |
| Runtime crates           | Identity, device, battery/network state, text clipboard, haptics, links, permissions, secure storage, private files, document import, settings, SQLite, localization, speech, wake locks, brightness, orientation leases, six sensors, crypto, logging, local authentication, foreground location and immediate notifications | See the operation-level [support matrix](plugin-inventory.md)                 |
| Scoped network plugins   | HTTP requests and WebSocket connection handles with generated async native registration and direct browser APIs                                                                                                                                                                                                               | Caller streaming, reconnect policy and advanced TLS configuration             |
| Direct host/browser APIs | Desktop/browser dialogs; explicitly configured autostart and host single-instance guard                                                                                                                                                                                                                                       | Dialog guest adapters and second-launch handoff                               |
| Host-configured APIs     | Scoped HTTP/WebSocket clients, child processes and desktop/browser dialogs; argument disclosure, global shortcuts, window placement, autostart and single-instance guard                                                                                                                                                      | Async guest adapters, broader native window support and second-launch handoff |

Generated native apps boot a Rust/Wasm interface and register selected Rust, Swift and Kotlin plugins. Sharing, store review and tracking transparency add focused platform actions to the runtime crates above. See [native application projects](host-projects.md) for build commands and host-owned configuration. A compiled handler can still return `Unavailable` on a platform without the underlying capability: macOS sensor and brightness handlers do not imply working desktop sensors or brightness control.

## Registration and execution

The host's selected `native.plugins` list must match runtime metadata on direct Cargo dependencies. Guest clients normally enable their crate's `guest` feature and cache a resolved numeric plugin ID. The installed host declares methods and grants access explicitly; operating-system permissions are a separate decision.

Crate names and runtime names are distinct: `leptoskit_plugin_local_authentication` resolves as `local-authentication`, and the manually installed `leptoskit_plugin_global_shortcut` handler resolves as `global-shortcut`. The new service names `arguments`, `shell`, `logging`, `notifications` and `location` match their crate suffixes. `window_state` is a host-only library with no runtime name or guest client.

Build browser adapters without `guest`; those clients call browser APIs directly. Crypto additionally requires its `web` feature for browser cryptographic randomness and UUID v4. An interpreter guest uses `guest` without `web`, obtains randomness from the native host, and must not import browser JavaScript. Hashing is available directly on all targets. Unsupported randomness returns an error rather than substituting a non-cryptographic generator.

Runtime metadata can name a Rust type (`rust-type`), an app-directory factory (`rust-factory`), an Apple module/type, or an Android handler. Shared native support packages are discovered through their `native-support` metadata. Android metadata can contribute manifest permissions, package-visibility queries and pinned `android-dependencies` Maven coordinates. Coordinates are validated, deduplicated and read as data by the generated Gradle host. `apple-usage-descriptions` declares required key names: iOS preparation checks that selected plugins have nonempty app-owned strings in `hosts/ios/Info.plist`. It never generates consent text. Entitlements and application-specific associations also remain explicit host configuration.

Presentation handlers use `apple-context = true` to receive a `NativePluginContext` with a weak reference to the owning controller. They do not search global windows. Android document import and sharing use `NativeActivityResultHost`; generated Activities provide the broker, forward results and retain cancelled presentation ownership until its terminal result. Stale callbacks cannot satisfy a later request. Custom hosts must implement these context and result contracts.

Select runtime names `document-picker`, `screen-orientation`, `light_sensor` and `pedometer`; they do not all follow the same crate-suffix spelling. Enable `guest` for interpreter clients and build direct browser adapters without it. Pedometer permission configuration remains app-owned: iOS requires `NSMotionUsageDescription`, Android requires `ACTIVITY_RECOGNITION`, and permission requests are explicit operations.

For asynchronous Rust handlers, `rust-async = true` selects a configuration factory and the host's async executor. The app supplies explicit policy in `native.plugin-options.<name>`; the factory validates those options before starting I/O. Network origins and permitted programs belong to host policy, never guest requests. See [host projects](host-projects.md) for the factory and feature contracts.

HTTP and WebSocket metadata use `rust-factory = "from_config"` with `rust-async = true`. Select `http` or `websocket` and supply exact `origins` in the matching host options. There is no wildcard or allow-all default. An origin grants every path on that origin; it is not an IP firewall or protection against DNS rebinding by an allowed host. Plain HTTP/WS requires an explicit entry and redirects are not followed. Factories reject unknown options and invalid limits before starting network I/O.

Interpreter dependencies use `default-features = false, features = ["guest"]`; the CLI enables `async-host` only for selected native builds. Browser apps retain the default `browser` feature and use direct adapters. Browser credentials, CORS, CSP and exposed response headers still follow browser policy. The default bridge and interpreter guest remain independent of Tokio; the optional native host creates a shared runtime only when an async plugin is installed.

`rust-worker = true` selects off-thread execution for a Rust handler. The generated registrant uses `app.install_worker`; settings, SQLite and logging use factories that retain app directories and defer disk access until a worker request. The filesystem factory establishes its directory capability during registration; subsequent file operations run on its worker. Crypto also selects worker dispatch. An owned worker retains the plugin and Rust payload without retaining the app or interpreter. Each plugin serializes its own invocations. Apple hosts use an `OperationQueue` with at most four active operations; blocking I/O does not run on Swift's cooperative executor. Android uses `Dispatchers.IO`. Replies return to the bridge's owning UI thread. Queued cancelled work is skipped; running synchronous I/O may finish and commit a side effect after the guest drops its future.

A synchronous Rust `Plugin::invoke` worker is not an asynchronous resource protocol. HTTP/WebSocket use the separate optional `AsyncPlugin` path: native jobs await I/O without blocking the interpreter, then transfer owned replies to its UI thread. Dialogs still require a live parent window and presentation lifecycle and do not have automatic guest registration. Autostart requires host-selected identity/executable/arguments, and the single-instance guard must be acquired before constructing the app.

Arguments require an explicitly disclosed snapshot. Global shortcuts require host construction after the platform event loop is ready, including the macOS main thread or Windows message loop; Linux support requires X11. Window state operates on the exact owned macOS window, with persistence and save/restore timing supplied by the application. Autostart requires host-selected identity/executable/arguments, and the single-instance guard must be acquired before constructing the app. These ownership requirements prevent a safe generic constructor from being inferred from the dependency alone.

Logging automatically registers a lazy app-directory worker, with files under the application's `logs` directory. It uses structured JSONL records, severity filtering, bounded rotation, exclusive writer ownership and explicit flush/close; file locking requires Rust 1.89. Its browser adapter writes to the console and provides no persistence guarantee. Local authentication, location and notifications have generated Apple/Android registration. Their permission prompts remain explicit operations; Face ID and location purpose strings, applicable entitlements and notification presentation policy remain application-owned.

## Generate a project

```sh
leptoskit generate app my-app --platform web
leptoskit generate app my-app-native --platform native
leptoskit generate plugin my-plugin
```

The plugin generator creates a standalone Cargo workspace with an echo method, registration metadata and an allocation-preservation test. Existing files are not overwritten. Custom Swift/Kotlin implementations must supply the corresponding native packages and handler metadata.

## Application-owned asynchronous services

A direct application dependency can declare an inert Rust service factory:

```toml
[package.metadata.leptoskit.runtime]
name = "application-network"
rust-service-factory = "native::from_app_dirs"
```

Select its name in `native.plugins`. The function has signature `fn(&Path, &Path) -> Result<ServiceInstaller, leptoskit_bridge::Error>`, receiving application data and cache directories. It must return an inert installer that owns any captured configuration. Generated Rust calls `app.defer_service(...)`; registration must not start tasks. `rust-type`, `rust-factory`, `rust-worker`, `apple-type` and `android-type` cannot accompany this field. One App accepts one service; compose several capabilities within that owner.

The returned installer receives a short actual-interpreter borrow and returns an owned `Box<dyn AppService>`. Platform code must establish thread-safe wake notification storage and a retained same-thread close scheduler before activation and guest start. A factory error must leave no running work; an activation error must leave no running work outside a returned owner. App construction can fail and synchronously drop a deferred installer. The installer cannot borrow the temporary directory arguments.

Poll on a real service wake or guest entry. `ServiceClose` owns task state without borrowing the interpreter; completion means native work is joined. Cancellation or Drop transfers a pending closing App to its retained scheduler, which must remain usable during constructor failure and view destruction. Never use an idle polling timer or detach native tasks.

This metadata generates Rust registration for macOS, iOS and Android. Installed platform runtime support also requires each platform's wake/close implementation. Linux/Windows runners retain their existing native-plugin restriction. The native application fixture under `crates/leptoskit_cli/tests/fixtures/app-service` separately exercises the generated constructor and asynchronous task lifecycle.

Solana SDK integration belongs in the application-owned service and remains off-chain client code. The generic LeptosKit CLI and platform do not acquire Solana dependencies. On-chain programs use Pina or official Solana program libraries.
