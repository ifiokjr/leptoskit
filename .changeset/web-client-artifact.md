---
"release": patch
---

# Web builds ship the app's own Wasm client

`leptoskit build` bound the first Wasm module cargo reported. When the client crate depended on another crate that also declares a `cdylib`, cargo compiled that dependency to Wasm first, and the build shipped the dependency's module under the app's name: the page loaded without an error, but the app's start function never ran. Web builds and native guest builds now take the module of the client package itself. This is what kept the live example on leptoskit.dev from responding to presses.
