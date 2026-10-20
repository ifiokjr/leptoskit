---
"release": minor
---

# Generate working iOS and Android applications

`leptoskit generate app --platform native` and `leptoskit new --platform native` now create UIKit and Android applications that load a Rust/Wasm interface and exercise Rust and native plugins. `leptoskit native prepare` generates host build state and plugin registrants from selected direct Cargo dependencies; `leptoskit native build` produces an iOS bundle or Android APK.

The app owns its platform sources, manifest and device tests. Generated state stays under `.leptoskit`. Platform dispatchers cancel pending work when the guest closes. Android supports arm64 and x86-64 targets with a pinned Gradle wrapper. Swift and Kotlin linting includes the hosts and templates.

The old Apple executable skeleton is replaced by mobile app projects. Render transactions currently use JSON; plugin transport remains binary. Desktop scaffolding, full widget coverage and the remaining plugin catalogs are still under development.
