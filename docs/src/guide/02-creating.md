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

| Flag                               | Default            | Meaning                                                           |
| ---------------------------------- | ------------------ | ----------------------------------------------------------------- |
| `--platform <web\|native\|server>` | `web`              | Sets `app.default_platform`. `native` also writes the Apple host. |
| `--name <NAME>`                    | the directory name | The app name. It must be a valid cargo package name.              |

`leptoskit new` never overwrites anything. If any file it would write already exists, it stops before writing a single one. Running `leptoskit new .` in an empty directory names the app after that directory.

## Native projects

```bash
leptoskit new my_app --platform native
```

A native project also gets its Apple host as a Swift Package in `hosts/apple`. There's no Podfile and no generated Xcode project: `open hosts/apple` opens it directly in Xcode, and `swift build` compiles it. [Host projects](../platforms/host-projects.md) explains the design.

The app's package id defaults to `com.leptoskit.<name>`, with everything except letters and digits removed from the name. That keeps it valid on both iOS, which forbids `_`, and Android, which forbids `-`. Set `native.package_id` in `leptoskit.toml` to use your own.
