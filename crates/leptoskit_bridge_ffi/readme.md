<p align="center">
  <a href="https://ifiokjr.github.io/leptoskit/"><img src="https://raw.githubusercontent.com/ifiokjr/leptoskit/main/docs/src/assets/mark.svg" alt="leptoskit" width="64"></a>
</p>

# leptoskit_bridge_ffi

> The C ABI, and the Swift, Kotlin, and C++ code, that generated leptoskit apps link to run a Wasm guest and draw it with native widgets.

A generated macOS, iOS, Android, or HarmonyOS app is a small native shell around Rust. The shell is Swift, Kotlin, or ArkTS, and it has to do three things the Rust code cannot do for it: draw the guest's widget tree with the platform's own views, run native plugin work (clipboard, notifications, file pickers) on the platform's threads, and keep ownership of every buffer that crosses the language boundary unambiguous. This crate is that boundary. `leptoskit native prepare` and `leptoskit build` copy its sources into the generated project, and its static library is what the shell links.

Most of the code is not Rust. The directory is the native half of the app host:

| Path                                              | What it is                                                                                                                                                                                                        |
| ------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `src/`                                            | The Rust `staticlib`/`rlib`. `lib.rs` exports the `lk_bridge_*` plugin functions over `leptoskit_bridge`. With the `app` feature, `app` and its siblings add the `lk_app_*` application runtime.                  |
| `include/`                                        | `leptoskit_bridge.h` is the ABI and the only place it is specified; `leptoskit_bridge.hpp` adds move-only C++20 wrappers; `module.modulemap` lets Swift import it as `CLeptosKitBridge`.                          |
| `apple/`                                          | The Swift plugin host (`LeptosKitBridge` in `Package.swift`): `NativePlugin`, `NativePluginContext`, job budgets, and app lifecycle, shared by iOS and macOS.                                                     |
| `android/`                                        | The Kotlin plugin host (`NativePlugin.kt`, the activity-result and permission hosts), `bridge.cpp` (the JNI layer), and `proguard-rules.pro` for release builds.                                                  |
| `harmonyos/`                                      | `bridge.cpp`, the experimental N-API layer for ArkTS.                                                                                                                                                             |
| `desktop/`                                        | The macOS renderer and view controller (`MacWidgetRenderer.swift`), which turns the guest's render transactions into AppKit views.                                                                                |
| `mobile/`                                         | The iOS and Android application runtime and renderers: `MobileWidgetRenderer.swift`, `NativeWidgetRenderer.kt`, `LeptosKitView.kt`, the Rust worker, update, and development-guest plumbing, and `app.cpp` (JNI). |
| `Package.swift`                                   | The SwiftPM package that builds `apple/` against the Rust library (`swift test --package-path crates/leptoskit_bridge_ffi -Xlinker -L -Xlinker target/debug`).                                                    |
| `*-tests/`, `ios-controls/`, `tests/`, `benches/` | The suites described under "Run the tests".                                                                                                                                                                       |

The Swift and Kotlin renderers in `desktop/` and `mobile/` are what generated apps use instead of the `leptoskit_appkit`, `leptoskit_uikit`, and `leptoskit_android` backend crates. They draw fewer widget kinds than those crates do; the platform pages in the book list the gaps, and they should be kept in step when the renderers change.

## How a call crosses

The ABI is single-threaded and non-reentrant. A host creates a bridge, registers plugins, and pumps requests:

```c
lk_bridge *bridge = lk_bridge_create();
int64_t plugin = lk_bridge_register(bridge, (const uint8_t *)"clipboard", 9, methods, 3);
lk_bridge_grant(bridge, (uint32_t)plugin, 0, true);

lk_request request;
if (lk_bridge_next_request(bridge, &request) == 1) {
    // ... do the native work, then hand the reply back:
    lk_bridge_complete(bridge, request.id, 0, reply, reply_len);
    lk_buffer_release(request.payload);
}
```

Every `lk_buffer` is Rust-owned memory that must be released exactly once, and `lk_bridge_complete_owned` hands a buffer back without copying. Native work may keep a buffer on a worker thread until it releases it. Statuses are the negative codes of `leptoskit_bridge::Error`; these differ from the Wasm-side statuses, which reserve `-1` for "pending". In practice Swift and Kotlin code does not call these functions by hand: the `NativePlugin` protocol/interface in `apple/` and `android/` does, and a plugin crate only supplies `invoke(method:payload:)`.

With `app` enabled, `lk_app_create_with_context` loads a guest bundle into the interpreter, `lk_app_start`, `lk_app_event`, and `lk_app_render` drive it, and `lk_app_begin_close` with `lk_app_close_poll` shuts it down in stages so native work cancels cleanly. The generated library supplies `leptoskit_app_create_with_context` and the plugin registrant.

## Features

| Feature      | What it adds                                                                                                                                                                                                         |
| ------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| _(default)_  | The `lk_bridge_*` plugin ABI only; depends on `leptoskit_bridge`.                                                                                                                                                    |
| `app`        | The application runtime: `leptoskit_wasm`, `leptoskit_native`, `leptoskit_yoga`, `leptoskit_codepush`, and the AppKit or UIKit backend.                                                                              |
| `async-host` | Implies `app`. Runs async plugin jobs on a Tokio runtime with cancellation (`leptoskit_bridge/async-host`).                                                                                                          |
| `devtools`   | Implies `app` and `async-host`. Reports each app to the DevTools hub that `leptoskit dev` runs (`lk_devtools_*`), with its guest's values through the `leptoskit.devtools` plugin; only `leptoskit dev` turns it on. |

Generated apps depend on this crate with `app` through the `leptoskit_bridge_ffi` entry the scaffold writes into their `Cargo.toml`; app authors do not add it themselves.

## Run the tests

Rust and the Swift, Kotlin, and C++ code are tested together, because the point is the boundary:

```sh
cargo test -p leptoskit_bridge_ffi --features async-host     # Rust-side ABI, ownership, worker, and update tests
devenv shell test:plugins:async     # async host jobs and cleanup (Rust)
devenv shell test:plugins:apple     # swift test against the built static library, plus every Apple plugin
devenv shell test:plugins:jni       # real Kotlin and JNI calls on the JDK: grants and direct buffer ownership
cargo bench -p leptoskit_bridge_ffi --features app --bench native_roundtrip   # Wasm guest to C ABI echo and back
```

- `apple-tests`, `apple-worker-tests`, and `controller-tests` are the SwiftPM test targets. The controller tests exercise the AppKit renderer, button and text painting, and the request executor, and build on macOS only.
- `android-tests` holds Kotlin and C++ tests for the Android host: service lifecycle and recovery, the activity-result broker, and `BridgeSmoke.kt`, which the JNI task compiles against `android/bridge.cpp` and runs on the JDK. `async-host-tests` holds the foreign-callback fixtures that the Rust `apple_async_lifecycle` and `jni_async_lifecycle` tests compile and drive.
- `ios-controls` builds a UIKit app that drives the real application controller against a deterministic C fixture, so teardown and update-recovery behavior is proven on a simulator without a Rust service. Its own `README.md` has the commands; CI runs it in the iOS simulator job.
- `tests/` holds integration tests for the app runtime, updates, development guests, host access, and ownership (some `--ignored` ones need a JDK or Apple toolchain).

The `native_roundtrip` benchmark measures the transport only. It excludes initialization, OS work, Swift or JNI dispatch, and UI scheduling, so it says nothing about application latency.

## Limits

- Every function is a trusted embedding interface; C pointers are not a boundary for untrusted guest pointers. The guest only ever sees the Wasm imports in `leptoskit_wasm`.
- The `app` feature builds the AppKit backend on macOS and the UIKit backend on iOS only; on Android the Kotlin renderer in `mobile/` draws the tree instead.
- HarmonyOS is experimental: the `harmonyos` workflow builds `harmonyos/bridge.cpp` into the app with the public SDK, but nothing runs on a device or emulator.
