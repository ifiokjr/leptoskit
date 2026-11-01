---
"release": minor
---

# Native apps report to DevTools under `leptoskit dev`

A macOS app that `leptoskit dev` launches now shows up in the DevTools hub beside the web page: its element tree, with each node's kind, classes, text, source line, and frame, and a commit for every render. The iOS Simulator and Android emulator apps run the same code. Selecting a node highlights it, and picking takes the next tap on a control instead of passing it to the app. When `leptoskit dev` reloads the guest, the app keeps its session and sends the new tree once it renders.

- **Only in development.** `leptoskit dev` builds the debug host with `leptoskit_bridge_ffi`'s new `devtools` feature and the guest with the app's `devtools` feature, and tells the app where the hub is: `LEPTOSKIT_DEVTOOLS` on macOS, `SIMCTL_CHILD_LEPTOSKIT_DEVTOOLS` in the iOS Simulator, and the `leptoskit.devtools` intent extra on Android, whose device reaches the hub through `adb reverse`. `leptoskit build` never does, and a release build refuses to report even with the feature.
- **Off the render path.** The app hands each render's changes to a thread of its own, which keeps a mirror of the tree, coalesces commits, and sends snapshots and patches. A second thread waits for the hub's commands. A hub that restarts gets a new session and the whole tree again.
- **For hosts.** `leptoskit_devtools` gains `Reporter`, `Mirror`, and `loopback_hub`, and its `HostAgent` waits out the hub's long-poll and reports `SessionGone` when the hub forgets a session. `leptoskit_bridge_ffi` exports `lk_devtools_hub`, `lk_devtools_highlight`, and `lk_devtools_picking`, which do nothing without the feature. A guest built with `devtools` sends each node's source location to its host, and `leptoskit_wasm::WasmHost::wire_node` reads it back.
