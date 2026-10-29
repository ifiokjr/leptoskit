---
"release": minor
---

# `leptoskit dev` runs every platform

`leptoskit dev` now runs every platform in `app.platforms` that this machine can run, and keeps them all current: the web app as before, plus each native app on this machine, in every booted iOS Simulator, and on every attached Android device or emulator. When no simulator or emulator is running, the newest iPhone simulator or the first installed emulator boots. Enabled platforms this machine can't run are skipped with the reason. `--platform`, repeatable, runs only the platforms named and fails when one can't run here. App logs stream into the same terminal, each line marked with its platform. `--device`, also repeatable, narrows iOS and Android to the simulators (by name or UDID) and adb serials named, booting a named simulator that is shut down. Devices that never finish booting or stop answering adb are skipped with the reason instead of stalling `dev`.

Each native app is built and installed once. After that, a change rebuilds only its WebAssembly interface, which `dev` serves from a loopback server (Android reaches it through `adb reverse`); the running app swaps it in without being reinstalled, keeping its window and its unchanged controls. A new interface that fails to start leaves the last working one on screen. Only debug builds accept an interface from `dev`: development interfaces are unsigned, so a release build refuses them.

Ctrl-C or `SIGTERM` stops `dev` cleanly, along with the app server and the desktop apps it opened.

Under that:

- `WasmHost::replace_guest` replaces a running guest while keeping the surface and the widgets it committed. The new guest's first commit is diffed against them.
- `NativeSurface::viewport` returns the viewport a surface lays out against.
- `leptoskit_axum::shutdown_signal`, also at `leptoskit::server::shutdown_signal`, resolves on Ctrl-C or `SIGTERM`, the signals `serve_listener` finishes its requests on.
- `leptoskit_codepush::dev` defines the development guest protocol, one long-polled `GET` per platform, with the `DevGuests` client and `follow_environment` for desktop runners behind the `http` feature. `leptoskit` exposes them through the new `codepush-http` feature.
- `leptoskit_bridge_ffi` adds `lk_dev_open`, `lk_dev_next`, `lk_dev_error`, and `lk_dev_destroy`. The generated iOS, macOS, and Android apps use them to follow `dev` without changes to their app-owned files.

Projects created before this release keep working. Their app-owned Linux and Windows runners open under `dev` but need a restart to show a change; to reload in place, copy the reload from a new project's `src/main.rs` and add `codepush-http` to the runner's `leptoskit` features.

`leptoskit native build ios` and `ios-simulator` remove `LD` and `LDPLUSPLUS` from Xcode's environment, so iOS builds work from a nix or devenv shell, which sets `LD=ld`.
