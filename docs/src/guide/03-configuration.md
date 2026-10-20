# Configuration

A leptoskit project is configured by one file, `leptoskit.toml`, at the project root. Every command looks for it in the current directory and then in each parent, so you can run `leptoskit` from anywhere inside a project.

Every section and every key is optional. Unknown keys are an error, so a typo fails loudly instead of being ignored.

```toml
[app]
name = "my_app"
version = "0.1.0"
default_platform = "web"
render = "islands"

[serve]
address = "127.0.0.1"
port = 3000

[build]
client_package = "my_app_client"
features = []
wasm_opt = true
rustflags = []

[native]
package_id = "com.example.myapp"
targets = ["macos", "ios", "android"]
viewport_width = 390.0
viewport_height = 844.0

[codepush]
enabled = true
server = "https://updates.example.com"
channel = "production"
mandatory = false
staging_dir = "codepush"
public_keys = [
	"3b6a27bcceb6a42d62a3a8d02a6f0d73653215771de243a63ac048a18b59da29",
]

[[plugins]]
name = "leptos_meta"
leptos = true
```

## `[app]`

| Key                | Default            | Meaning                                                         |
| ------------------ | ------------------ | --------------------------------------------------------------- |
| `name`             | the directory name | The app name, used for the package id and release names.        |
| `version`          | `"0.1.0"`          | The app's semantic version.                                     |
| `default_platform` | `"web"`            | What `dev` and `build` target: `"web"`, `"native"`, `"server"`. |
| `render`           | `"csr"`            | How the web build renders: `"csr"`, `"ssr"`, or `"islands"`.    |

### Render modes

`render` decides the cargo features each side of a web build gets, following the leptos convention that every leptos app already uses:

| `render`    | Server package          | Client package              | Who serves the app                      |
| ----------- | ----------------------- | --------------------------- | --------------------------------------- |
| `"csr"`     | no mode feature         | no mode feature             | leptoskit's static server over `dist/`  |
| `"ssr"`     | `ssr`                   | `hydrate`                   | your server binary, rendering each page |
| `"islands"` | `ssr`, `leptos/islands` | `hydrate`, `leptos/islands` | your server binary, rendering each page |

The server-rendering modes need the server package to declare an `ssr` feature and the client to declare a `hydrate` feature, and `islands` needs both to depend on `leptos` directly. When one is missing, the build stops and tells you which package needs what. The [small bundles guide](07-small-bundles.md) walks through each mode.

## `[serve]`

| Key       | Default       | Meaning                                                           |
| --------- | ------------- | ----------------------------------------------------------------- |
| `address` | `"127.0.0.1"` | The IP address to bind. Use `"0.0.0.0"` to listen on all of them. |
| `port`    | `3000`        | The port to bind.                                                 |

## `[build]`

| Key              | Default       | Meaning                                                                                                                            |
| ---------------- | ------------- | ---------------------------------------------------------------------------------------------------------------------------------- |
| `client_package` | auto-detected | The wasm client crate. When unset, leptoskit uses the workspace member with a `cdylib` target; set it when there is more than one. |
| `features`       | `[]`          | Cargo features enabled for app builds.                                                                                             |
| `wasm_opt`       | `true`        | Whether release builds run `wasm-opt` over the wasm bundle.                                                                        |
| `rustflags`      | `[]`          | Extra `rustc` flags for app builds.                                                                                                |

## `[native]`

| Key               | Default                       | Meaning                                                                                                                                                  |
| ----------------- | ----------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `package_id`      | `com.leptoskit.<name>`        | The reverse-domain application id. Each dot-separated segment is letters and digits, starting with a letter, so the same id is valid on iOS and Android. |
| `targets`         | `["macos", "ios", "android"]` | Native shells to build: any of `macos`, `ios`, `android`, `windows`, `linux`.                                                                            |
| `viewport_width`  | `390.0`                       | The initial viewport width for the headless layout pass.                                                                                                 |
| `viewport_height` | `844.0`                       | The initial viewport height for the headless layout pass.                                                                                                |

## `[codepush]`

| Key           | Default        | Meaning                                                                                                                                                 |
| ------------- | -------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `enabled`     | `false`        | Whether the app checks for over-the-air updates.                                                                                                        |
| `server`      | `""`           | The release registry's base URL. Required when `enabled = true`.                                                                                        |
| `channel`     | `"production"` | The release channel this build tracks.                                                                                                                  |
| `mandatory`   | `false`        | Whether updates install without asking.                                                                                                                 |
| `staging_dir` | `"codepush"`   | Where updates are staged, relative to the app data directory.                                                                                           |
| `public_keys` | `[]`           | The hex-encoded ed25519 publisher keys whose signed releases the app installs. At least one is required when `enabled = true`. List two to rotate keys. |

## `[[plugins]]`

Each `[[plugins]]` table turns on one plugin.

| Key       | Default | Meaning                                                                     |
| --------- | ------- | --------------------------------------------------------------------------- |
| `name`    | —       | The plugin name. Names without a `path` resolve against the built-ins.      |
| `path`    | none    | A path to a local plugin crate.                                             |
| `leptos`  | `false` | Marks a leptos plugin, which is passed to the app build as a cargo feature. |
| `options` | `{}`    | A table of options handed to the plugin.                                    |

## Using the configuration from Rust

`leptoskit_config` is the library behind the file, so tools and build scripts read exactly what the CLI reads:

```rust,no_run
use leptoskit_config::LeptoskitConfig;

let (config, root) = LeptoskitConfig::load().expect("a leptoskit.toml in this directory or a parent");
println!("{} at {}", config.app.name, root.display());
```
