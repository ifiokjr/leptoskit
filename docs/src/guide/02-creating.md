# Creating a project

```bash
leptoskit new my_app
cd my_app
leptoskit dev
```

`leptoskit dev` builds the project, serves it on <http://127.0.0.1:3000>, and rebuilds whenever a file changes.

## What you get

`leptoskit new` writes a small, complete project that builds as it is:

```text
my_app/
├── leptoskit.toml        # the project configuration
├── Cargo.toml            # the server package, and the workspace
├── src/main.rs           # the server binary
├── client/
│   ├── Cargo.toml        # the wasm client: a cdylib
│   └── src/lib.rs        # functions exported to the page
├── public/index.html     # the page that loads the client
└── .gitignore
```

The client crate's `crate-type = ["cdylib"]` is how `leptoskit build` finds it, so you don't need to configure it. Its `wasm-bindgen` dependency is pinned to the exact version built into the CLI, because wasm-bindgen only accepts wasm compiled against its own version.

The starter counter uses leptoskit's shared component builders and built-in Tailwind classes. The client renders the shared tree through the optional Leptos web adapter. The [shared component guide](07-shared-components.md) shows how to use the same screen with a native surface.

## Options

| Flag                               | Default            | Meaning                                                            |
| ---------------------------------- | ------------------ | ------------------------------------------------------------------ |
| `--platform <web\|native\|server>` | `web`              | Sets `app.default_platform`. `native` writes iOS and Android apps. |
| `--name <NAME>`                    | the directory name | The app name. It must be a valid cargo package name.               |

`leptoskit new` never overwrites anything. Web/server generation checks every destination file before writing. Running `leptoskit new .` in an empty directory names the web app after that directory. Native generation requires a new directory.

## Native projects

```bash
leptoskit new my_app --platform native
```

A native project contains UIKit and Android application sources, a Rust/Wasm guest, and a Rust static library. `leptoskit native build ios-simulator` generates an Xcode application using Swift Package plugins; `leptoskit native build android` builds an APK through the bundled Gradle wrapper. Selected plugins register automatically from Cargo metadata. [Host projects](../platforms/host-projects.md) describes toolchain setup, registration, signing and device tests.

The app's package id defaults to `com.leptoskit.<name>`, with everything except letters and digits removed from the name. That keeps it valid on both iOS, which forbids `_`, and Android, which forbids `-`. Set `native.package_id` in `leptoskit.toml` to use your own.
