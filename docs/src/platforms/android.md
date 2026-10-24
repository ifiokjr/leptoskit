# Android

The Android runtime probe exercises the shared Rust layout, renderer, wasm runtime, and update client. [Generated native apps](host-projects.md) render the guest's tree with Android views, and can replace that guest through [code push](host-projects.md#code-push).

## Architecture

```
Android views (android.view.View hierarchy)
   ▲ mutations (create / update props / insert / layout)
AndroidBackend (leptoskit_android, the NativeBackend over the View system)
   ▲
leptoskit_native renderer                              ─┐
leptoskit_yoga layout                                    ├─ shared, pure Rust
leptoskit_wasm runtime (wasmi interpreter)               │
leptoskit_codepush client                               ─┘
   ▲ your app's wasm bundle
```

`leptoskit_yoga` is an in-house Rust layout engine. Native hosts share that implementation. The Leptos web adapter emits CSS and uses browser layout instead.

## Shipping

Same model as iOS. The store artifact carries the shell: an APK or AAB with a small Kotlin activity that starts the runtime. Interface versions ship over the air through code push, with binary deltas. Updates are downloaded, tried, and confirmed exactly as on iOS, and CI runs the same journey on an emulator. New native capabilities, such as plugins, permissions, or new `Custom` widget kinds, go in a store release with a new `native.update_contract`. Everything else rides the channel.

```bash
leptoskit native bundle --out dist
leptoskit codepush release --channel production
```
