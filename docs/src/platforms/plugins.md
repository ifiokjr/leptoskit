# Runtime plugins

On iOS, Android, and the other native platforms, your app's code runs as WebAssembly inside the host. That keeps it portable and lets [code push](../guide/05-code-push.md) replace it, but WebAssembly can't call UIKit, Android's Java APIs, or any other operating system library directly. A **runtime plugin** bridges that gap: a crate with a Rust API your app calls, and a native implementation that the host runs on its behalf.

leptoskit ships plugins for the clipboard, device and app information, battery and network state, how a foldable is folded, haptics, links, permissions, secure storage, files, SQLite, settings, localization, speech, keeping the screen awake, brightness, orientation, six sensors, cryptography, logging, biometric authentication, location, notifications, sharing, store review, tracking transparency, HTTP, WebSockets, and desktop services such as dialogs, menus, and the tray. The [plugin support matrix](plugin-inventory.md) says what each one does on each platform.

Runtime plugins are different from the `[[plugins]]` table in `leptoskit.toml`, which turns on build plugins.

## Use a plugin

Here is the clipboard plugin in an app named `water`. It needs three changes.

**1. Call it from your app.** Add the crate to `water_app/Cargo.toml`, and turn on its `guest` feature for native builds, which is how the app talks to the host:

```toml
[dependencies]
leptoskit_plugin_clipboard = "0.1"

[features]
native = ["leptoskit/guest", "leptoskit_plugin_clipboard/guest"]
```

Both calls are `async` and return `Result<_, leptoskit_bridge::Error>`. In a native build, the `guest` module sends the call to the host:

```rust
let clipboard = leptoskit_plugin_clipboard::guest::Clipboard::new()?;
clipboard.write_text("Copied from leptoskit").await?;
```

In a browser build, the `web` module calls the browser's Clipboard API directly:

```rust
leptoskit_plugin_clipboard::web::write_text("Copied from leptoskit").await?;
```

The `guest` module only exists when the `guest` feature is on, and the `web` module only in a browser build without it, so put each call behind `#[cfg(feature = "native")]` or `#[cfg(all(feature = "web", target_arch = "wasm32"))]`.

**2. Install it in each host.** Add the same crate to the `Cargo.toml` of each platform folder that should have it, such as `ios/Cargo.toml` and `android/Cargo.toml`. That is what brings in the Swift and Kotlin code.

**3. Grant it.** List its runtime name in `leptoskit.toml`:

```toml
[native]
plugins = ["clipboard"]
```

The next native build registers the plugin with the host and grants your app its methods. A plugin that is a host dependency but isn't listed stays unregistered, and one that is listed but missing from the host stops the build. [Host projects](host-projects.md#automatic-plugin-registration) covers the details.

Granting a plugin doesn't grant operating-system permissions. Plugins that need one, such as location or the camera, say which `Info.plist` strings, manifest permissions, or entitlements to add, and your app still asks the user at run time.

## Reactive settings and secrets

The settings and secure-storage plugins can also back `leptoskit_use`'s async storage hooks, so one shared screen persists state the same way on every platform. Enable the crate's `async-storage` feature, wrap the platform store, and hand it to `use_async_storage`:

**1. Add the feature in `water_app/Cargo.toml`:**

```toml
[features]
native = [
	"leptoskit/guest",
	"leptoskit_plugin_store/guest",
	"leptoskit_plugin_store/async-storage",
]
```

**2. Wrap the store at startup.** On a native build the guest client implements the contract directly; the browser wraps its local-storage store:

```rust
#[cfg(all(feature = "web", target_arch = "wasm32"))]
let store = leptoskit_plugin_store::StoreStorage::<Settings>::new(
    leptoskit_plugin_store::web::Store::open()?,
);
```

**3. Persist through the hook.** `set` and `remove` commit before the signal updates, and a failed write keeps the last committed value:

```rust
let preferences = use_async_storage(store, "profile", Settings::default()).await?;
(preferences.set_value)(updated).await?;
```

The secure-storage plugin ships the same adapter for the OS credential store: values are JSON secrets under a service you choose, bounded by the plugin's 2 KiB limit, so use it for tokens rather than bulk settings.

## Runtime names

Most runtime names match the crate's suffix: `leptoskit_plugin_clipboard` is `clipboard`. These don't:

| Crate                                    | Runtime name            |
| ---------------------------------------- | ----------------------- |
| `leptoskit_plugin_document_picker`       | `document-picker`       |
| `leptoskit_plugin_keep_awake`            | `keep-awake`            |
| `leptoskit_plugin_local_authentication`  | `local-authentication`  |
| `leptoskit_plugin_screen_capture`        | `screen-capture`        |
| `leptoskit_plugin_screen_orientation`    | `screen-orientation`    |
| `leptoskit_plugin_secure_store`          | `secure-store`          |
| `leptoskit_plugin_store_review`          | `store-review`          |
| `leptoskit_plugin_tracking_transparency` | `tracking-transparency` |

`light_sensor` and `image_manipulator` keep their underscores. `dialog`, `menu`, `tray`, `global_shortcut`, `window_state`, `autostart`, `single_instance`, and `arguments` have no runtime name: the desktop host constructs them itself, because each needs something only the host has, such as a running event loop or a window.

## Plugins that need a policy

Plugins that reach the network or start processes do nothing until the host gives them a policy. Your app's code can't widen it:

```toml
[native]
plugins = ["http", "websocket"]

[native.plugin-options.http]
origins = ["https://api.example.com"]

[native.plugin-options.websocket]
origins = ["wss://live.example.com"]
```

An origin allows every path on it. There is no wildcard: plain `http://` and `ws://` need their own entries, and redirects are not followed. An origin is not a firewall, and doesn't protect against DNS rebinding by a host you allowed. In the browser, the HTTP and WebSocket plugins use `fetch` and `WebSocket`, so CORS, CSP, and the browser's own rules apply instead.

## Notes on specific plugins

- **Crypto:** random bytes and UUIDs in the browser need the crate's `web` feature. Native apps get randomness from the host. Hashing works everywhere. A platform without secure randomness returns an error rather than a weaker generator.
- **Image manipulator:** crop, resize, rotate, flip, and re-encode JPEG and PNG, up to 16 steps per request. Images are limited to 4 MiB in and out, four million pixels, and 8,192 pixels per side, and the dimensions are checked before any pixels are decoded. Requires Rust 1.88.
- **Screen capture:** on Android, it can block screenshots and recordings (`FLAG_SECURE`) while your app holds a lease, and on Android 14 or newer it reports screenshots. On iOS it reports recording and screenshots but can't block them. It never reads pixels, and it can't undo a screenshot it reports.
- **Logging:** writes structured JSON lines under the app's `logs` directory, with severity filtering and rotation. In the browser it writes to the console only. File locking requires Rust 1.89.
- **Pedometer:** iOS reports steps for an interval, Android a running count since boot; they aren't the same number. It needs `NSMotionUsageDescription` on iOS and `ACTIVITY_RECOGNITION` on Android.
- **Desktop services:** global shortcuts need the platform's event loop running, and need X11 on Linux. Window state works on the app's own macOS window. Autostart and the single-instance guard take the app's identity from the host, and the guard must be acquired before the app starts.
- **Sensors and brightness on macOS:** the handlers compile and register, but return `Unavailable` where the Mac has no such hardware or control.

Cancelling a plugin call stops it if it hasn't started. A call already inside the operating system may still finish, so cancelling doesn't undo a write.

## Write a plugin

```sh
leptoskit generate plugin my-plugin
```

This creates a standalone Cargo workspace for a plugin with one `echo` method, its registration metadata, and a test. It never overwrites existing files. Add Swift and Kotlin handlers when the plugin needs native code; the [metadata reference](host-projects.md#plugin-metadata-reference) lists every way a plugin can register.

## Application-owned asynchronous services

Sometimes an app needs long-running native work of its own, such as a sync engine or a blockchain client, rather than a reusable plugin. Your app can declare one **service** in a crate it depends on directly:

```toml
[package.metadata.leptoskit.runtime]
name = "application-network"
rust-service-factory = "native::from_app_dirs"
```

List its name in `native.plugins`. The factory has the signature `fn(&Path, &Path) -> Result<ServiceInstaller, leptoskit_bridge::Error>` and receives the app's data and cache directories. It must not start anything: it returns an inert installer that owns its configuration. The host later activates the installer, which returns a `Box<dyn AppService>`. An app has at most one service factory, so put related work in one service. A service factory cannot be combined with `rust-type`, `rust-factory`, `rust-worker`, or `rust-async`.

The same runtime package can also declare `apple-module`, `apple-type`, and `android-type` when its service needs a separate native plugin. On macOS and iOS, the generated host copies and imports a declared Apple package and registers its plugin; on Android, it copies the declared Kotlin sources and registers that plugin. Context, permission, dependency, and usage-description metadata work as for other native plugins. A destination without a declared native plugin installs only the Rust service. For example, an Android wallet approval plugin can accompany a service that also works on iOS without that platform capability.

The metadata `name` selects the package in `native.plugins`; it does not rename its capabilities. The Rust service registers its capabilities through the host, and each native `NativePlugin.name` supplies its own distinct bridge identity. For example, one package named `wallet` can register `wallet-session` from Rust and `wallet-platform` from Swift or Kotlin. Reusing a capability name fails bridge registration; it never replaces an existing handler. A Rust-only service needs no platform type.

A service runs when the host wakes it or your app calls in; never poll on an idle timer or detach native tasks. Closing joins its native work. Registration code is generated for macOS, iOS, and Android; the Windows and Linux hosts don't run services yet.

For example, a Solana client belongs in a service like this, as ordinary off-chain Rust; leptoskit itself has no Solana dependency.
