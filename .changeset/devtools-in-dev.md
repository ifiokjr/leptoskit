---
"release": minor
---

# Turn DevTools on in `leptoskit dev`

Apps opt in with one feature, `devtools = ["leptoskit/devtools"]`, and one line at startup, `leptoskit::devtools::install(env!("CARGO_PKG_NAME"))`. New projects have both. `leptoskit dev` then compiles the web client with the feature and the hub's address, so the page reports itself to the hub; `leptoskit build` never does, and refuses a release build whose `build.features` asks for it. `leptoskit doctor` points out a client crate without the feature.

Every element now has a match key, built from where it was created and its place among its siblings, and the web and native renderers compute the same key for the same element, so one node can be found on every platform. The web agent sends a snapshot only when the page changes, picks without the click reaching the app, and reconnects when the hub restarts.

Components such as `View()`, `ScrollView()`, `Heading()`, the `view!` tags, `DataTable`, and `VirtualList` now record the app's line that called them, rather than a line inside leptoskit, so the DevTools tree points at your code.
