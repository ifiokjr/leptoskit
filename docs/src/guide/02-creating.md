# Creating a project

Create one application for web, iOS, Android, macOS, Windows, and Linux:

```bash
leptoskit new amazing
cd amazing
leptoskit dev
```

The starter serves a counter at <http://127.0.0.1:3000>. The root server renders the same application that the browser hydrates and native hosts run as Wasm.

## Work in the shared app

`leptoskit new amazing` generates this workspace:

```text
amazing/
├── Cargo.toml
├── leptoskit.toml
├── src/
│   └── main.rs
├── amazing_app/
│   ├── Cargo.toml
│   ├── build.rs
│   └── src/
│       ├── lib.rs
│       ├── routes.rs
│       ├── routes/
│       ├── components/
│       └── services/
├── amazing_backend/
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs
│       └── rpc/
├── amazing_shared/
│   ├── Cargo.toml
│   └── src/lib.rs
├── web/
├── ios/
├── android/
├── macos/
├── windows/
├── linux/
├── assets/
└── .gitignore
```

Put routes, components, client state, and application services in `amazing_app`. The starter counter uses the shared component builders and Tailwind classes. See [shared components](07-shared-components.md).

Put RPC handlers and server-only dependencies in `amazing_backend`. Shared response models and domain types belong in `amazing_shared`. Both the app and backend depend on the shared crate. The root `src/main.rs` starts the SSR server, mounts the backend's routes, and renders the app. The app does not depend on the backend implementation.

Keep platform startup, permissions, packaging, and native integrations in the top-level platform folders. Each native folder contains an app-owned Rust host package. The CLI writes generated projects and intermediate files into the ignored `.leptoskit/` directory when you build.

## Build a platform

All six platforms are enabled in `app.platforms`. The default build targets the web with SSR. Each build selects one application platform feature:

```bash
leptoskit build --platform web
leptoskit build --platform macos
leptoskit native build ios-simulator
leptoskit build --platform android
leptoskit build --platform windows
leptoskit build --platform linux
```

Native builds need the selected platform's SDK on a supported host. Creating the workspace does not require those SDKs. See [host projects](../platforms/host-projects.md) for native setup.

To change the default destination when creating a project, pass `--platform`:

```bash
leptoskit new amazing --platform ios
```

The option changes `app.default_platform`. All six platform folders and their shared app are still generated. Use `--name <NAME>` to choose an app name independently of its directory. During SDK development, use `--sdk-path /path/to/leptoskit` to depend on local crates.

Generation refuses to overwrite existing files. In an empty directory, `leptoskit new .` takes the app name from that directory.

## Add a platform dependency

The application declares `web`, `ios`, `android`, `macos`, `windows`, and `linux` Cargo features, with no default platform feature. Add an optional dependency to the feature that needs it:

```toml
[dependencies]
serde_json = { version = "1", optional = true }

[features]
ios = ["leptoskit/ios", "native", "dep:serde_json"]
```

Keep the generated feature entries when extending them. Native application code still compiles to `wasm32-unknown-unknown`. Its dependencies must support Wasm, including dependencies selected by the `ios` feature. Put UIKit, JNI, and other OS-native dependencies in the platform's host package or a native plugin instead.

The CLI selects the destination feature explicitly. Native app builds also select the generated `native` feature, which enables the SDK's guest adapter. SSR selects the web platform and server rendering features. Do not enable multiple platform features in one application build.

## Override a module for one platform

The starter's build script enables platform module selection:

```rust
fn main() -> Result<(), leptoskit_build::Error> {
	leptoskit_build::platforms()
}
```

Declare a module with `platform_module!` to let the selected platform replace its shared file:

```rust
leptoskit::platform_module!(pub mod home = "routes/home");
```

The path is relative to the app crate's `src` directory and has no extension. Keep a shared file, then add overrides where needed:

```text
amazing_app/src/routes/
├── home.rs
├── home.ios.rs
└── home.android.rs
```

An iOS build uses `home.ios.rs`. An Android build uses `home.android.rs`. Other builds use `home.rs`. Callers use the same `crate::routes::home` module in every build. Keep the public items that callers need in every implementation.

An ordinary `mod home;` keeps Rust's standard module resolution. Use `platform_module!` for each module that needs suffix overrides. The build script watches the source directory, so adding or removing an override triggers another Cargo build.
