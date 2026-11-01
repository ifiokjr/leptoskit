---
"release": minor
---

# DevTools shows a native app's route and state

`track_router`, `expose`, `track_form`, and `track_queries` now report from the macOS, iOS, and Android apps `leptoskit dev` runs, not only from the browser. A native app's values appear in the same session as its element tree, and **Navigate**, editing state, and refetching, invalidating, or editing a query in DevTools change the running app, which draws the result. When `leptoskit dev` reloads the interface, the panels show the new interface's values instead of the old ones.

- **No setup.** The hooks start reporting on their own in a native app; `leptoskit::devtools::install` only matters before the first one. Commonplace's macOS, iOS, and Android apps now show its route and store, as its website does.
- **`leptoskit_plugin_devtools`** is the new bridge plugin the values travel through. The shared app runtime installs it whenever an app reports to DevTools, which only `leptoskit dev` builds do, so apps never select it. Its calls are bounded: reports of at most 1 MiB, split when larger, and at most 64 waiting commands.
- **For hosts.** `leptoskit_bridge_ffi`'s `devtools` feature now implies `async-host`. `leptoskit_devtools` gains `Reporter::with_guests`, which opens the session once the guest has said what it can feed and lists that in `Hello.capabilities`, and `Source::guest`, the handle the plugin reports through. Its new `guest` feature, which `leptoskit`'s `guest` and `devtools` features turn on together, is the agent inside the guest.
