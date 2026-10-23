---
"release": minor
---

# Generate one application for web and native platforms

`leptoskit new` creates a workspace with a shared application, backend, and shared domain crate, plus web, iOS, Android, macOS, Windows, and Linux runners. All six platforms are enabled by default. Builds select a platform feature and isolate its generated state and application bundle.

Builds select registered `.platform.rs` module overrides and compile the shared application into a browser bundle or native Wasm guest. App-owned platform runners keep native dependencies separate from the application. Native code-push releases use a channel for each platform so incompatible runtime shells cannot block one another's updates.
