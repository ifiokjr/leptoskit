# Plan: every plugin on every platform

Status: **proposed**. Nothing here is implemented. Decisions marked **(open)** need the maintainer's answer before the phase that depends on them starts; the recommended answer is listed first.

Written against `main` at `a8d5049` (30 October 2026). Line numbers drift; search for the named function if a reference has moved.

## The problem

Runtime plugins are meant to be cross-platform, but today the platform an app targets decides whether a plugin can be used at all:

- **Windows and Linux can't use any runtime plugin.** Listing one in `native.plugins` stops the build: "the windows runner does not support native runtime plugins yet" (`crates/leptoskit_cli/src/native/rust_desktop.rs:40`). The generated registrant is an empty `configure` function (`rust_desktop.rs:68`). Seven plugins already have working Windows/Linux Rust code that no generated app can reach.
- **HarmonyOS can use two plugins.** `validate_plugins` (`native/harmonyos.rs:33`) accepts only synchronous Rust plugins (`os`) and ArkTS plugins (`posture` is the only one). Worker plugins are rejected. Async plugins (`http`, `shell`, `websocket`) pass validation, build, and then tear the page down on their first call because `bridge.cpp` can't dispatch them.
- **A plugin that lacks one platform fails the whole build** on that platform (`validate_platform_plugin`, `native/project.rs:481`), so an app can't use the same `leptoskit.toml` everywhere.
- **HarmonyOS builds compile desktop Linux code.** `aarch64-unknown-linux-ohos` reports `target_os = "linux"`, and nothing in the workspace checks `target_env = "ohos"`, so every Linux dependency table applies (arboard, rfd, wayland-backend, vendored libdbus, keyring with openssl, global-hotkey, ksni…). The `os` plugin reports `linux` on HarmonyOS.
- **Where a plugin works is written down in four places that disagree:** `Cargo.toml` target tables, `#[cfg]` code, the `[package.metadata.leptoskit.runtime]` fields, and the CLI's own per-host rules. `docs/src/guide/19-localization.md:161` tells authors to grant localization on Windows and Linux, which fails the build.

## The goal

Two promises, delivered in this order:

1. **Every platform can use any plugin.** An app lists plugins once, and builds and runs on iOS, Android, macOS, Windows, Linux and HarmonyOS. Where a platform has no implementation yet, the build warns and calls from the app fail with `Unavailable`, the same error every plugin already returns for unsupported cases.
2. **Every plugin works on every platform where the OS can support it.** Missing implementations are written plugin by plugin. A small number stay `Unavailable` because the hardware or concept doesn't exist (a barometer on most desktops, App Tracking Transparency outside Apple).

### Not in scope

- Code push for Windows, Linux and HarmonyOS (separate work).
- The `router` crate, which is a build plugin, not a runtime plugin.
- Turning the desktop-only libraries (`menu`, `tray`, `dialog`, `single_instance`, `window_state`) into bridge plugins. Phase 2 optionally adds registration metadata for the three that already implement `Plugin` (`arguments`, `autostart`, `global_shortcut`).

## How it works today

Read this before changing anything. Paths are under `crates/` unless stated.

### A plugin call

1. Guest code calls `Client::new(NAME)` (`leptoskit_bridge/src/guest.rs:165`), which calls the host's `resolve` import (`leptoskit_wasm/src/plugins.rs:49`). **If no host registered that name, `resolve` returns `Unavailable` and no request is ever queued.** This means "no implementation on this platform" needs no stub type: leaving the plugin unregistered already gives guests the right error.
2. `submit` (`plugins.rs:70`) runs `Bridge::check_call` (`leptoskit_bridge/src/lib.rs:372`): unknown method → `Unavailable`, not granted → `PermissionDenied`, no slot → `Busy`.
3. The host drains requests after each guest entry point and completes them; `WasmHost::wake_plugins` (`leptoskit_wasm/src/lib.rs:773`) runs the guest's `leptoskit_plugins_ready` export.

`Bridge` has no locking and lives on the UI thread inside the wasmi store (`leptoskit_wasm/src/lib.rs:251`). `replace_guest` (dev reload) creates a new `Bridge`, so call IDs restart at 1; anything completing after a reload must check `HostIdentity`.

### Who registers what

| Host                | Built from                                                                                                                        | Rust plugins                                                                                                            | Native plugins         |
| ------------------- | --------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------- | ---------------------- |
| iOS, macOS          | `leptoskit_bridge_ffi::App` (`app.rs:148`, fixed to `WasmHost<HeadlessBackend>`) + Swift                                          | `install`, `install_worker` (Swift `OperationQueue`, 4 wide), `install_async` (tokio, `async_jobs.rs`), `defer_service` | Swift `apple-type`     |
| Android             | same `App` + Kotlin                                                                                                               | same; Kotlin worker pool and async glue                                                                                 | Kotlin `android-type`  |
| HarmonyOS           | same `App` + `bridge.cpp` over NAPI                                                                                               | sync `install` only; no worker, async or service glue in `bridge.cpp`                                                   | ArkTS `harmonyos-type` |
| Windows, Linux, TUI | `WasmHost<WindowsBackend/GtkBackend/…>` directly, in app-owned runners (`templates/WindowsMain.rs`, `LinuxMain.rs`, `TuiMain.rs`) | none                                                                                                                    | none                   |

The CLI picks a registration per plugin in `register_rust_plugin` (`native/project.rs:354`) and `uses_platform_dispatcher` (`native/registry.rs:154`). Rust registration, when present, wins on every platform; native dispatch is used only when there is no Rust registration (or for hybrid services).

### Plugin inventory

48 crates: 29 native (Swift + Kotlin), 10 pure Rust, 9 with no runtime metadata.

| Group                                      | Plugins                                                                                                                                                                                                                                                                                            | Windows/Linux code today    | HarmonyOS today                          |
| ------------------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------- | ---------------------------------------- |
| Rust, sync                                 | `os`                                                                                                                                                                                                                                                                                               | yes (runner can't register) | works, but reports `linux`               |
| Rust, worker                               | `crypto`, `filesystem`, `image_manipulator`, `logging`, `sqlite`, `store`                                                                                                                                                                                                                          | yes (runner can't register) | rejected                                 |
| Rust, async                                | `http`, `shell`, `websocket`                                                                                                                                                                                                                                                                       | yes (runner can't register) | passes validation, crashes on first call |
| Native, desktop `Plugin` impl exists       | `battery`, `clipboard`, `device`, `linking`, `localization`, `network`, `secure_store`                                                                                                                                                                                                             | Rust impl, unregistered     | none                                     |
| Native, desktop code without `Plugin` impl | `document_picker`, `keep_awake`, `notifications`                                                                                                                                                                                                                                                   | Rust API only               | none                                     |
| Native, no desktop code                    | `accelerometer`, `application`, `barometer`, `brightness`, `gyroscope`, `haptics`, `light_sensor`, `local_authentication`, `location`, `magnetometer`, `pedometer`, `permissions`, `posture`, `screen_capture`, `screen_orientation`, `sharing`, `speech`, `store_review`, `tracking_transparency` | none                        | `posture` only (ArkTS)                   |
| No runtime metadata                        | `arguments`, `autostart`, `global_shortcut` (have `Plugin` impls); `dialog`, `menu`, `single_instance`, `tray`, `window_state` (direct API); `router` (build plugin)                                                                                                                               | direct use                  | n/a                                      |

## Decisions

### P1. One function decides a plugin's implementation per target

Replace `uses_platform_dispatcher`, `validate_platform_plugin`, HarmonyOS `validate_plugins`, and the blanket rejection in `rust_desktop::generate` with one resolver in `native/registry.rs`:

```rust
pub enum Implementation<'a> {
    Rust(&'a RustRegistration),          // rust-type / rust-factory, worker or async flavour
    Apple { module: &'a str, ty: &'a str },
    Android { ty: &'a str },
    ArkTs { ty: &'a str },
    Hybrid { service: &'a str, native: Box<Implementation<'a>> },
}

impl Plugin {
    /// What runs `self` on `target`, or why nothing does.
    pub fn implementation(&self, target: Target, host: &HostCapabilities)
        -> Result<Implementation<'_>, Missing>;
}

pub enum Missing {
    NoImplementation,                     // the plugin has nothing for this platform
    HostCannotRun(&'static str),          // e.g. "the HarmonyOS host doesn't run worker plugins yet"
}
```

`HostCapabilities` says what each generated host can run (sync, worker, async, service, native kind). Today: iOS/macOS/Android run everything; HarmonyOS runs sync + ArkTS; Windows/Linux/TUI run nothing. Each phase widens it. All generators (Swift, Kotlin, ArkTS, desktop Rust, mobile Rust) iterate the resolved implementations instead of re-deriving them.

### P2. Per-platform registration in plugin metadata

Plugins need to say "Swift on Apple, Rust on Windows and Linux". Add optional per-platform tables. Resolution order for a target: its platform table, then the shared Rust fields, then the platform's native field (`apple-*` for iOS and macOS, `android-*`, `harmonyos-*`), else `Missing::NoImplementation`.

```toml
[package.metadata.leptoskit.runtime]
name = "clipboard"
apple-module = "LeptosKitClipboard"
apple-type = "ClipboardPlugin"
android-type = "dev.leptoskit.plugins.clipboard.ClipboardPlugin"
android-context = true

[package.metadata.leptoskit.runtime.windows]
rust-factory = "desktop::Clipboard::from_context"

[package.metadata.leptoskit.runtime.linux]
rust-factory = "desktop::Clipboard::from_context"
```

- Allowed table names: `ios`, `macos`, `android`, `harmonyos`, `windows`, `linux`, `tui`.
- Allowed keys inside a table: `rust-type`, `rust-factory`, `rust-worker`, `rust-async`, and `unavailable = true` (explicitly opt a platform out, which silences the warning).
- Keep `deny_unknown_fields` on the table struct.
- Existing shared Rust fields keep meaning "everywhere", so `os`, `crypto`, etc. need no change.
- The supported-platform list is **derived** from this resolution, not declared separately. (This replaces the hand-written `platforms = [...]` field suggested in earlier discussion: deriving it means it can't drift.)

### P3. A missing implementation warns; it doesn't fail **(open)**

- **Recommended:** default to a warning, with `native.missing-plugins = "error"` for apps that want the old strictness. The warning names the plugin, the platform and the reason, for example: `warning: haptics has no Linux implementation; calls fail with Unavailable`.
- Alternative: keep failing by default and add `"warn"` as an opt-in.

Missing implementations are simply not registered (see "A plugin call"). Malformed metadata (an Apple module without a type, an unknown table key) stays a hard error.

`native.missing-plugins` is a configuration change: update `leptoskit_config`, then `devenv shell schema:update`.

### P4. A shared, backend-generic plugin host for Rust plugins

Windows/Linux/TUI runners can't use `leptoskit_bridge_ffi::App`, because it's fixed to `WasmHost<HeadlessBackend>` and its worker and async machinery is private and reachable only through the C ABI. Move that machinery into a new pure-Rust crate, **`leptoskit_plugin_host`** (name **(open)**):

- `PluginHost`: install (sync), install_worker, install_async (behind `async-host`), and later defer_service.
- Worker pool: fixed `std::thread`s (4), per-plugin `Mutex`, cancel flag checked before and after, `catch_unwind`. This is the body of `lk_worker_execute` (`bridge_ffi/src/app.rs:498`), moved.
- Async: `AsyncHost`, `admit`, `execute`, `runtime()` moved from `bridge_ffi/src/app/async_jobs.rs` (same limits: 128 jobs, 32 MiB, 8 MiB per reply, 2+4 tokio threads).
- Completions go into an `mpsc` channel tagged with `HostIdentity`; the host calls a runner-supplied `wake: Arc<dyn Fn() + Send + Sync>` to get the UI loop to drain.
- `pump<B: NativeBackend>(&mut self, host: &mut WasmHost<B>)`: drain cancellations, run sync plugins inline, dispatch worker and async calls, apply ready completions, call `wake_plugins` only if the guest exports `leptoskit_plugins_ready` (`wake_plugins` errors otherwise). Bounded per turn (64, as `lk_app_next_request` does).

`leptoskit_bridge_ffi` switches to the moved code with no behaviour change, so iOS, Android and macOS keep one implementation.

### P5. macOS keeps Swift **(open)**

- **Recommended:** macOS keeps registering the Swift implementations, which are the reference and usually richer. The desktop Rust code is registered on Windows and Linux only.
- Alternative: register Rust on macOS too, and delete the Swift desktop paths.

Once everything else is done, drop `macos` from the Rust-only dependency tables where nothing uses them, to cut dependencies.

### P6. Services on desktop wait

No in-repo plugin uses `rust-service-factory`. `ServiceInstaller` and `AppService` are typed to `WasmHost<HeadlessBackend>` (`bridge_ffi/src/app/services.rs:48`). Desktop and HarmonyOS hosts report `HostCannotRun("services")` until a plugin needs one; then do Phase 2.5.

### P7. TUI gets plugins with Windows and Linux **(open)**

- **Recommended:** yes. `TuiHost` uses the same `WasmHost`, and its loop can take the same wake channel.
- Alternative: no; it reports `HostCannotRun`.

### P8. Platform concepts that don't exist stay `Unavailable` **(open)**

- **Recommended:** `tracking_transparency` outside Apple, `store_review` on Linux, and `haptics`, `posture` and `screen_orientation` on desktop Linux get `unavailable = true` in their metadata. They return `Unavailable`, with no invented "authorized" answers.
- Alternative: return a platform-neutral status, for example "not required" for tracking.

### P9. Windows APIs that need package identity **(open)**

Some WinRT APIs need the app to have package identity (MSIX): toast notifications with actions, `StoreContext` reviews, and some sensor and location consent flows. Check each one before porting.

- **Recommended:** implement what works unpackaged, and return `Unavailable` with a documented reason otherwise.
- Alternative: add MSIX packaging to the Windows build first.

## Phases and pull requests

Each item is one PR. Branch prefixes follow `AGENTS.md`. Every PR includes a changeset (`monochange run change …`) when the change is visible to users, updated docs, and green `devenv shell fix:all`, `lint:all` and `test:all` (with `CARGO_BUILD_JOBS=4`). "Verify" says where it can be checked: **local** means the Mac doing the work, and **CI** means the checks on the PR.

```
1.1 ──► 1.2 ──┬──► 2.1 ──► 2.2 ──► 2.3 ──► 4.x
              │                 └──► 2.4 (optional)
              ├──► 3.1 ──► 3.2 ──► 3.3…3.7
              └──► 5.1
6.1, 6.2… (independent; any time)
```

After 1.2 merges, the Windows/Linux track (2, 4) and the HarmonyOS track (3) touch different files and can run in parallel, for example one on each machine.

### Phase 1: every plugin builds everywhere

#### 1.1 `fix/harmonyos-not-linux`: stop treating HarmonyOS as Linux

- Change every `target_os = "linux"` and `cfg(unix)` gate in plugin `Cargo.toml` tables and `#[cfg]`s that means "desktop Linux" to `all(target_os = "linux", not(target_env = "ohos"))`. There are 19 plugin manifests with Linux or Unix tables (inventory in the HarmonyOS research: autostart, battery, clipboard, device, dialog, document_picker, global_shortcut, keep_awake, linking, localization, logging, menu, network, notifications, secure_store, shell, single_instance, tray, filesystem). For `cfg(unix)` uses that only need `libc`, keep them.
- `shell` must not compile process spawning for HarmonyOS. It answers `Unavailable` there.
- `os` reports `harmonyos` when `target_env = "ohos"`.
- Also check `leptoskit` (`crates/leptoskit/Cargo.toml`, the `leptoskit_gtk` table) and the backends for the same mistake.
- Tests:
  - A unit test in `os`.
  - A CI step in `.github/workflows/harmonyos.yml` that builds a fixture host depending on **every** runtime plugin crate, with none selected, for `aarch64-unknown-linux-ohos`. This proves the crates compile.
- Verify:
  - CI (the SDK is on the Ubuntu runner).
  - Locally, `cargo check --target aarch64-unknown-linux-ohos -p <pure-Rust crate>` once the target is installed. Crates with C build scripts need the SDK, so only CI can check those.
- Docs: `docs/src/platforms/harmonyos.md` (shell is unavailable). Changeset: patch, `release` group.

#### 1.2 `feat/plugin-platform-resolution`: per-platform metadata and missing-means-warning

- `native/registry.rs`:
  - Add the `PlatformRegistration` table struct (P2) to `Plugin`.
  - Add `Plugin::implementation` and `HostCapabilities` (P1).
  - Validate table names and keys, and the Rust path rules (reuse `rust_service_path`-style checks).
- Replace these call sites with the resolver:
  - `project.rs` (`register_rust_plugin`, `validate_platform_plugin`, `android_registrant`, iOS/macOS generation in `project/ios.rs` and `desktop.rs`)
  - `harmonyos.rs` (`validate_plugins`, `generate_plugins`)
  - `rust_desktop.rs` (remove the blanket rejection; Windows/Linux/TUI report `HostCannotRun` until 2.2)
- `leptoskit_config`: add `native.missing-plugins`, then run `schema:update`.
- Print warnings once per build per plugin. `leptoskit dev` shows them in its output.
- Fixes the HarmonyOS async crash: async and worker plugins now resolve to `HostCannotRun` there, so they're skipped with a warning.
- Tests:
  - Update the six tests that assert today's failures:
    - `rust_desktop.rs:176`
    - `project/tests.rs:436`
    - `project/hybrid_tests.rs:53`
    - `project/hybrid_tests.rs:103`
    - `harmonyos.rs:427`
    - `desktop/tests.rs:96`, which keeps failing because the metadata is malformed
  - New tests:
    - table precedence
    - an unknown table key fails
    - `unavailable = true` silences the warning
    - `missing-plugins = "error"` fails
    - HarmonyOS skips `http` with a warning
    - an Android build skips an Apple-only plugin
- Docs:
  - `host-projects.md` metadata reference (89–128) and automatic registration (59–73)
  - `plugins.md:47` (no longer "stops the build")
  - `guide/03-configuration.md` (the new key)
  - `harmonyos.md:20`
  - Fix `guide/19-localization.md:161`.
- Verify: local (`cargo test -p leptoskit_cli`); CI.

### Phase 2: Windows, Linux and the terminal run plugins

#### 2.1 `refactor/plugin-host-crate`: move the Rust plugin host out of the FFI crate

- Create `crates/leptoskit_plugin_host` (P4) and move the worker execution and `async_jobs.rs` logic into it, with a safe Rust API.
- `leptoskit_bridge_ffi` calls into it; its C ABI and behaviour are unchanged.
- `host_features` (`native/registry/async_plugins.rs:42`) enables `async-host` on the crate that now owns the runtime.
- Tests:
  - Move the existing bridge_ffi worker and async tests with the code.
  - Add `PluginHost::pump` tests against a `WasmHost<HeadlessBackend>` guest fixture: a sync, a worker and an async plugin; cancellation; an identity mismatch after `replace_guest`.
- Verify:
  - Local: `cargo test -p leptoskit_plugin_host -p leptoskit_bridge_ffi`; `test:plugins:apple`; Android `test:plugins:jni` if the Android SDK is installed.
  - CI for everything.

#### 2.2 `feat/desktop-runner-plugins`: generated Windows, Linux and terminal apps register Rust plugins

- `rust_desktop.rs`:
  - Generate a registrant with a new function: `pub fn plugins(host: &mut PluginHost, context: &DesktopContext) -> Result<(), Error>`. Reuse `register_rust_plugin`, refactored to take the receiver and context expressions.
  - Keep generating the old `configure(&mut WasmHost<B>)` as a no-op, so runners that apps already own keep compiling. When an older runner is detected, print a one-time hint pointing at the migration note.
  - The registrant is `include!`d by both the runner's `main.rs` and its `src/lib.rs` staticlib (`new_project.rs:753`). Make it compile in both, or stop including it in `lib.rs`.
- `rust_desktop::build`: pass `registry.host_features` (today it builds with `--no-default-features` and no features).
- `DesktopContext`:
  - Per-app data and cache directories from `dirs`, named by `native.id` (the reverse-domain id), created on startup.
  - Match the layout `secure_store` already uses on desktop.
- Templates (and their copies in `examples/file_routes/{linux,windows}`, `examples/opencode/tui`, `benchmarks/arena/apps/leptoskit/{linux,windows}`):
  - **Windows** (`templates/WindowsMain.rs`): the wake posts `WM_APP` to the window (`PostMessageW` with the HWND from `window.handle()`, wrapped to be `Send`). In `run_frames`, call `plugins.pump(&mut host)` after `pump_messages` and before `wait_for_messages`, and after `start` and `reload`.
  - **Linux** (`templates/LinuxMain.rs`): the wake uses `glib::MainContext::default().invoke` (gtk4 0.9 / glib 0.20 has no `MainContext::channel`). Pump after the event listener, resize and scale callbacks, the dev-reload timeout, and `start`.
  - **TUI** (`templates/TuiMain.rs`), if P7 is yes: send a wake into `TuiHost::run`'s loop.
- Dev reload: before `replace_guest`, cancel in-flight async jobs, set worker cancel flags, and drop completions whose `HostIdentity` no longer matches. Rebuild plugin instances with `plugins(...)` again.
- `HostCapabilities` for Windows/Linux/TUI: sync, worker and async.
- Tests:
  - Registrant unit tests in `rust_desktop.rs`.
  - Extend `tests/dev_reload.rs` with a worker call in flight during a reload.
  - A fixture app (or extend the existing native plugin suite) selecting `os`, `crypto` (worker), `http` (async, against a local server) and `haptics` (expected `Unavailable`), run by `ci.yml` `applications` on Windows and `coverage` on Linux.
- Docs:
  - `windows-linux.md`: a new "Plugins" section explaining what works, the migration note for runners created earlier, and the data directory.
  - `plugins.md`, `host-projects.md`, the `plugin-inventory.md` legend, `docs/src/readme.md` "Where things stand".
- Verify:
  - Windows: CI only. A Mac can't run Win32.
  - Linux: locally in a container. Docker and podman are on Mini 02; use an Ubuntu 22.04 image with GTK 4 dev packages and Xvfb, matching `native-ui.yml`. Linux is also checked in CI.

#### 2.3 `feat/desktop-plugin-registration`: register the desktop code that already exists

- Add `windows` and `linux` metadata tables (P2), plus a `from_context` factory where construction can fail, to:
  - `battery`, `clipboard`, `device`, `linking`, `localization`, `network`, `secure_store`
  - Battery and network register their snapshot methods; their wait methods return `Unavailable` until implemented.
- Write `Plugin` wrappers, then register:
  - `keep_awake`: a lease per call, released on cancel or drop.
  - `notifications`: D-Bus on Linux; on Windows, see P9.
  - `document_picker`: async, over `dialog`'s desktop dialogs.
- Optional: desktop metadata tables for `arguments`, `autostart` and `global_shortcut`, whose `Plugin` impls already exist. Mobile reports them missing.
- Tests: each plugin's existing unit tests, plus the 2.2 fixture extended with clipboard, localization and device.
- Docs:
  - `plugin-inventory.md` rows (Windows/Linux becomes "Auto")
  - each plugin's `readme.md` ("automatic Apple/Android registration" becomes the real list)
  - guide pages that name platforms
- Verify: CI for Windows; a container for Linux.

#### 2.4 `feat/desktop-plugin-services` (only when a plugin needs it)

Make `ServiceInstaller` and `AppService` generic over `B: NativeBackend` (`bridge_ffi/src/app/services.rs`). Drive `AppService::poll` from the runner, with a `ServiceWake` that posts the UI wake. Add a UI-thread `CloseScheduler` (a GTK idle source, or a check in the Win32 loop).

### Phase 3: HarmonyOS reaches parity

Can run in parallel with Phase 2 once 1.2 has merged. There is no HarmonyOS device or emulator in CI, so every ArkTS port is built against the real SDK and tested with Deno fakes; the docs must say "not yet checked on a device" until someone checks one.

#### 3.1 `feat/harmonyos-worker-async`: the HarmonyOS host runs worker and async Rust plugins

- `leptoskit_bridge_ffi/harmonyos/bridge.cpp` `Render`:
  - Try `lk_app_worker` and `lk_app_async` before the ArkTS check at `:167`.
  - Run workers with `napi_create_async_work`, at most 4 at a time.
  - Return async completions to the UI thread with `napi_create_threadsafe_function`.
  - Then `lk_worker_complete` → `lk_app_wake` → `Render`.
  - Map cancelled IDs to `lk_worker_cancel` / `lk_async_cancel`.
  - After `close()`, release late results and never complete them.
- Rendering without a call: today a `RenderBatch` reaches the page only as the return value of a NAPI call. Add an `onBatch` callback, or an event that makes the page `pump()`, in `templates/harmonyos/index.d.ts` and `Index.ets`.
  - Apps own these files, so write a migration note.
  - The CLI should detect an old `Index.ets` and keep worker and async plugins at `HostCannotRun` for that app.
- `HostCapabilities` for HarmonyOS: sync, worker, async and ArkTS.
- Toolchain proof: extend the 1.1 fixture so CI builds `crypto`, `http` (tokio, rustls/ring) and `sqlite` (bundled C) for OHOS. None of these have been tried.
- Tests: `harmonyos.rs` generation tests; the SDK build test (`tests/harmonyos_sdk.rs`) with a worker and an async plugin selected.
- Docs: `harmonyos.md` table and plugin lifecycle; `docs/agents/harmonyos.md` evidence.
- Verify: CI (`harmonyos.yml`). Locally only if the OpenHarmony SDK is installed (`OHOS_NDK_HOME`); it isn't on Mini 02 today.

#### 3.2 `feat/harmonyos-plugin-permissions`: permissions metadata

Add `harmonyos-permissions` (like `android-permissions`). The CLI merges the requested `ohos.permission.*` entries into `entry/src/main/module.json5`. That file is scaffolded once and owned by the app, so the CLI updates only a marked block and errors on conflicts. Needed before haptics (`VIBRATE`), location and similar plugins.

#### 3.3–3.7 ArkTS ports, in batches

For each plugin:

- `harmonyos/<Type>.ts` implementing `NativePlugin` (`templates/harmonyos/plugins.ts:67`). Copy `name` and `methods` from the Rust constants, and use the exact wire format with little-endian `DataView`.
- `harmonyos/<x>_test.ts` with fakes under `harmonyos/testing/`, and a byte-for-byte wire-format test against Rust. Use `posture` as the template: its `PosturePlugin.ts`, `posture_test.ts` and `deno.json`.
- `harmonyos-type` (and `harmonyos-context` and permissions as needed) in its metadata.
- A `deno test` line in `harmonyos.yml` and `devenv.nix` `test:harmonyos`, plus the plugin added to the SDK build fixture.

Candidate APIs are listed below. Check each against the plugin's actual methods before starting.

| Batch           | Plugins                                                                                         | HarmonyOS kits to check                                                                                                                                                                          |
| --------------- | ----------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| 3.3 sensors     | accelerometer, gyroscope, magnetometer, light_sensor, barometer, pedometer                      | `@ohos.sensor`                                                                                                                                                                                   |
| 3.4 device      | application, device, battery, network, localization, brightness, keep_awake, screen_orientation | `@ohos.bundle.bundleManager`, `@ohos.deviceInfo`, `@ohos.batteryInfo`, `@ohos.net.connection`, `@ohos.i18n`, window brightness, `window.setWindowKeepScreenOn`, `window.setPreferredOrientation` |
| 3.5 interaction | clipboard, haptics, linking, sharing, speech, notifications                                     | `@ohos.pasteboard`, `@ohos.vibrator`, `UIAbilityContext.openLink`, `@kit.ShareKit`, `@kit.CoreSpeechKit`, `@ohos.notificationManager`                                                            |
| 3.6 security    | secure_store, local_authentication, permissions                                                 | `@kit.AssetStoreKit` / HUKS, `@ohos.userIAM.userAuth`, `abilityAccessCtrl`                                                                                                                       |
| 3.7 the rest    | location, document_picker, screen_capture, store_review, tracking_transparency                  | `@ohos.geoLocationManager`, `@ohos.file.picker`, `window.setWindowPrivacyMode`; store review and tracking → `unavailable = true` unless AppGallery has a fitting API                             |

### Phase 4: write the missing Windows and Linux implementations

There's one PR per plugin or small group. Each one adds a `desktop` module (or Windows-only and Linux-only modules), a `Plugin` impl, the metadata tables, unit tests runnable on that OS in CI, the plugin's readme, and its `plugin-inventory.md` row. Order them by how much apps need them. Check every row against P9 first.

| Plugin                                                                     | Windows candidate                                                      | Linux candidate                                  |
| -------------------------------------------------------------------------- | ---------------------------------------------------------------------- | ------------------------------------------------ |
| application                                                                | generated from `leptoskit.toml` at build time (same for every desktop) | same                                             |
| speech                                                                     | `Windows.Media.SpeechSynthesis`                                        | speech-dispatcher                                |
| location                                                                   | `Windows.Devices.Geolocation`                                          | GeoClue2 over D-Bus                              |
| sharing                                                                    | `DataTransferManager` share UI                                         | `unavailable = true` (no standard)               |
| local_authentication                                                       | Windows Hello `UserConsentVerifier`                                    | `unavailable = true`                             |
| permissions                                                                | capability access status for camera and microphone                     | portal camera access where available             |
| brightness                                                                 | WMI or monitor configuration (laptops)                                 | logind `SetBrightness` / `/sys/class/backlight`  |
| screen_capture                                                             | `SetWindowDisplayAffinity(WDA_EXCLUDEFROMCAPTURE)`                     | `unavailable = true`                             |
| accelerometer, gyroscope, magnetometer, light_sensor, barometer, pedometer | `Windows.Devices.Sensors` (devices with sensors)                       | iio-sensor-proxy (accelerometer, light, compass) |
| screen_orientation                                                         | `SetDisplayAutoRotationPreferences`                                    | `unavailable = true`                             |
| store_review                                                               | `StoreContext` (needs MSIX, see P9)                                    | `unavailable = true`                             |
| haptics, posture, tracking_transparency                                    | `unavailable = true`                                                   | `unavailable = true`                             |

### Phase 5: the support table writes itself

#### 5.1 `feat/plugin-support-report`

- Add `leptoskit plugins` (and `--json`). It prints each dependency plugin's resolved implementation per platform, using `Plugin::implementation`.
- Generate the platform columns of `docs/src/platforms/plugin-inventory.md` from that output, keeping the hand-written "implemented subset" and "remaining scope" columns.
- Add a check to `lint:all` that fails when the generated part is stale.
- Add a HarmonyOS column to the inventory.
- Update `docs/agents/plugin-port-inventory.md`.

### Phase 6: cross-building and one name for "platform" (independent)

From the earlier investigation. These items don't block the phases above and can be scheduled separately.

- **6.1 `feat/cli-cross-build`:**
  - Replace `check_host` (`native/mod.rs:257`) and `dev/native.rs:321` `support` with checks for the tools each target needs: `cargo-xwin` or MSVC for Windows, a GTK 4 sysroot or container for Linux, `xcrun` for Apple.
  - Add `--arch`, defaulting from the target, not the build machine (`Target::rust`, `native/mod.rs:87`).
  - Make `legacy_target` stop depending on the machine.
- **6.2 `feat/platform-cfg`:**
  - Add `leptoskit_build::plugin()`, called from each plugin's `build.rs`. It sets `cfg(platform = "ios" | … | "harmonyos" | "web")` plus the groups `apple`, `desktop` and `mobile`, only when the target matches _and_ the crate's feature for that platform is on. It declares the values with `cargo::rustc-check-cfg` so typos warn.
  - `leptoskit_build::platforms()` sets the same cfg for app code.
  - Pilot on clipboard and haptics.
- **6.3:**
  - Move the remaining plugins to `cfg(platform = …)`.
  - Add a lint that rejects raw `target_os`/`cfg(unix)` in plugin code, and any manifest spelling outside the approved set (the approved Linux spelling is the one 1.1 introduces).
  - Add per-platform plugin features (`default = [all]`) so apps can leave a platform out.
- **6.4:** CI jobs that cross-build Windows from Linux with `cargo-xwin`, and Android from Linux (already partly there).

## Risks

- **Event-loop integration on Windows and Linux** is the least-trodden part. Keep `pump` bounded, never call `wake_plugins` on a guest without the export, and test reload with work in flight.
- **App-owned files don't update themselves.**
  - Desktop runners (`main.rs`) and HarmonyOS `Index.ets` / `index.d.ts` are copied once.
  - Each phase that changes them keeps the old shape compiling, detects it, and prints a migration hint. It never edits app-owned files silently.
- **Moving the async and worker host (2.1) touches iOS, Android and macOS.** Keep it a pure move, landed and green on its own before 2.2 builds on it.
- **OHOS toolchain** for tokio, ring, rusqlite and dbus is untested. 1.1 and 3.1 prove it in CI before any port depends on it.
- **Windows package identity (P9)** may push some APIs to `Unavailable` or to MSIX packaging.
- **No HarmonyOS device in CI.** Docs must say which ports haven't been checked on a device.
- **Warnings can be ignored.** The default change in P3 trades build failures for runtime `Unavailable`. The `leptoskit plugins` report (5.1) and `missing-plugins = "error"` give strict apps a way back.

## Working on Mini 02

This work moves off Mini 03 to keep its disk free.

- Mini 02: macOS arm64, 116 GB free (30 Oct 2026), T3 running, checkout at `~/Developer/projects/leptoskit/leptoskit` on `main`, with `devenv`, `gh`, `deno`, `xcodebuild`, `docker` and `podman`.
- Not installed there: `cargo-xwin`, the Android SDK (`ANDROID_HOME`), and the OpenHarmony SDK (`OHOS_NDK_HOME`).
- Mini 01 has 71 GB free. It could take the HarmonyOS track (Phase 3) in parallel once 1.2 has merged.

What each phase can check locally on a Mac:

| Phase         | Locally on Mini 02                                        | Needs CI                                |
| ------------- | --------------------------------------------------------- | --------------------------------------- |
| 1.1           | pure-Rust `cargo check` for OHOS once the target is added | crates with C build scripts (SDK)       |
| 1.2, 5.1      | all CLI tests                                             | —                                       |
| 2.1           | crate tests, Apple plugin tests                           | Android JNI unless the SDK is installed |
| 2.2, 2.3, 4.x | Linux in an Ubuntu container (GTK 4 + Xvfb)               | Windows, always                         |
| 3.x           | Deno ArkTS tests                                          | SDK HAP builds                          |

Disk hygiene:

- `CARGO_BUILD_JOBS=4`.
- At most two worktrees with build output at once.
- `cargo clean` (or delete `target/`) in a worktree once its PR merges.
- Container images for Linux checks are shared, not rebuilt per PR.

Process:

- Rebase onto `main` before each PR; other agents push often.
- Commits are GPG-signed.
- Squash-merge once green.
- Dependent PRs (2.1 → 2.2 → 2.3) can go in a `gh stack`.

## Open decisions

| ID | Question                                               | Recommended                                                              |
| -- | ------------------------------------------------------ | ------------------------------------------------------------------------ |
| P3 | Missing implementation: warn or fail by default?       | Warn, with `native.missing-plugins = "error"` available                  |
| P4 | Name of the new crate                                  | `leptoskit_plugin_host`                                                  |
| P5 | macOS: keep Swift, or switch to the Rust desktop code? | Keep Swift                                                               |
| P7 | Terminal apps get plugins with Windows and Linux?      | Yes                                                                      |
| P8 | Concepts that don't exist on a platform                | `Unavailable`, never invented answers                                    |
| P9 | Windows APIs needing MSIX                              | Implement what works unpackaged; document the rest                       |
| —  | Order of the Windows/Linux and HarmonyOS tracks        | Windows/Linux first; HarmonyOS in parallel on a second machine after 1.2 |
