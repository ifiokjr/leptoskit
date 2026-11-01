---
"release": minor
---

# See plugin and server calls in DevTools

The Calls panel now lists what your app calls while it runs. On macOS, iOS, and Android it shows each call to a native plugin, such as the clipboard or the secure store, with its status and duration. On every platform, an app using `leptoskit::rpc` shows each call to its server, one row per retry. Native apps also name each element's `#[component]`, as the web already did.

An element whose text or color comes from a signal kept the line that built it only until the signal changed; it now keeps it.
