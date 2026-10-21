# leptoskit_plugin_application

Installed host application identity: ID, localized display name, release version, and build number. Apple implementations read `Bundle.main`; Android reads the running application's `PackageManager` record. This is the host's identity even when the interpreted guest is updated independently.

```rust,ignore
let app = leptoskit_plugin_application::guest::Application::new()?;
let identity = app.snapshot().await?;
```

Add this crate as a direct dependency and `"application"` to `native.plugins` for automatic Apple/Android registration. On Rust desktop hosts, construct `Application::new(&Snapshot { ... })` with the host's packaging metadata. This validates and caches the encoded reply. The library does not use its own Cargo package name or version as the app's identity. Apple desktop bundles use the same Swift implementation as iOS.

Browsers expose no equivalent installed-native-app identity. Browser applications can provide their own explicitly configured `Snapshot`; document titles, origins, and this library's version are not substituted for native package metadata. Missing fields are `None`. No device IDs, vendor IDs, install referrers, release channel guesses, installation dates, or store environment APIs are included.

The initial port covers the four identity constants from [Expo Application](https://docs.expo.dev/versions/latest/sdk/application/).
