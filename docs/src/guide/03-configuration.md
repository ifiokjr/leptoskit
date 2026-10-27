# Configuration

A leptoskit project is configured by one file, `leptoskit.toml`, at the project root. Every command looks for it in the current directory and then in each parent, so you can run `leptoskit` from anywhere inside a project.

Every section and every key is optional. Unknown keys are an error, so a typo fails loudly instead of being ignored. New projects include an editor schema directive and `schema_version = 0`. This schema is unstable: breaking changes are expected until the first version 1 schema.

```toml
#:schema https://ifiokjr.github.io/leptoskit/schemas/leptoskit.v0.schema.json

schema_version = 0

[app]
name = "my_app"
version = "0.1.0"
default_platform = "web"
platforms = ["web", "ios", "android", "macos", "windows", "linux"]
render = "islands"

[serve]
address = "127.0.0.1"
port = 3000

[build]
client_package = "my_app_app"
features = []
rustflags = []

[bundle]
optimize = "smallest"
split = true
wasm_opt = true
precompress = true
budget_kib = 80

[native]
package_id = "com.example.myapp"
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

| Key                | Default                                                  | Meaning                                                                                |
| ------------------ | -------------------------------------------------------- | -------------------------------------------------------------------------------------- |
| `name`             | the directory name                                       | The root server package name, also used for the package id and release names.          |
| `version`          | `"0.1.0"`                                                | The app's semantic version.                                                            |
| `default_platform` | `"web"`                                                  | The destination for `build`. `dev` serves web applications.                            |
| `platforms`        | `["web", "ios", "android", "macos", "windows", "linux"]` | Enabled application destinations. Each build selects one destination.                  |
| `render`           | `"csr"`                                                  | How the web build renders: `"csr"`, `"ssr"`, or `"islands"`. New projects set `"ssr"`. |

`platforms` lists the platforms you can build: `leptoskit build --platform <platform>` refuses one that isn't listed, before compiling anything. `default_platform` is the one `leptoskit build` uses without `--platform`, and it must be in `platforms`. The list can't be empty or repeat an entry.

Listing a platform doesn't compile anything for it. Each build turns on one platform feature in your app crate, such as `ios`, and with it that platform's [per-platform files](02-creating.md#use-a-different-file-on-one-platform).

`"harmonyos"` is also accepted, for the experimental [HarmonyOS](../platforms/harmonyos.md) starter. It is not in the default list, because the six-platform project has no HarmonyOS app. Older projects may also use `"native"`, which builds the platform in `native.targets`, or `"server"`, which builds only the web server.

### Render modes

`render` decides the cargo features each side of a web build gets, following the leptos convention that every leptos app already uses:

| `render`    | Server package          | Client package              | Who serves the app                      |
| ----------- | ----------------------- | --------------------------- | --------------------------------------- |
| `"csr"`     | no mode feature         | no mode feature             | leptoskit's static server over `dist/`  |
| `"ssr"`     | `ssr`                   | `hydrate`                   | your server binary, rendering each page |
| `"islands"` | `ssr`, `leptos/islands` | `hydrate`, `leptos/islands` | your server binary, rendering each page |

The server-rendering modes need the server package to declare an `ssr` feature and the client to declare a `hydrate` feature, and `islands` needs both to depend on `leptos` directly. When one is missing, the build stops and tells you which package needs what. The [small bundles guide](09-small-bundles.md) walks through each mode.

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
| `rustflags`      | `[]`          | Extra `rustc` flags for app builds.                                                                                                |

## `[bundle]`

What a release build ships: how hard the wasm client is optimized, how the output is compressed, and how much it may weigh. The [small bundles guide](09-small-bundles.md#release-builds) explains each setting.

| Key             | Default  | Meaning                                                                                                                                                                                    |
| --------------- | -------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `optimize`      | `"fast"` | How hard release builds optimize the client: `"fast"` (your `release` profile, then `wasm-opt`) or `"smallest"` (adds fat LTO and one codegen unit). Override per build with `--optimize`. |
| `cargo_profile` | none     | A cargo profile from your `Cargo.toml` to compile the release client with, in place of the one `optimize` picks.                                                                           |
| `split`         | `true`   | Split web `#[lazy]` code, lazy routes, and `#[island(lazy)]` into wasm chunks that load on demand. Clients without lazy code skip splitting.                                               |
| `wasm_opt`      | `true`   | Whether release builds run `wasm-opt -Oz` over the wasm bundle.                                                                                                                            |
| `precompress`   | `true`   | Whether release web builds write brotli (`.br`) and gzip (`.gz`) copies next to every compressible file.                                                                                   |
| `budget_kib`    | none     | The most the initial load may weigh, in KiB, brotli-compressed. Release builds over it fail.                                                                                               |

## `[native]`

| Key               | Default                       | Meaning                                                                                                      |
| ----------------- | ----------------------------- | ------------------------------------------------------------------------------------------------------------ |
| `package_id`      | `com.leptoskit.<name>`        | The reverse-domain application id used by native hosts.                                                      |
| `plugins`         | `[]`                          | Names of runtime plugins installed from direct dependencies of the selected host package.                    |
| `targets`         | `["macos", "ios", "android"]` | The native platform for the HarmonyOS starter and older projects. Six-platform projects use `app.platforms`. |
| `viewport_width`  | `390.0`                       | The initial viewport width for the headless layout pass.                                                     |
| `viewport_height` | `844.0`                       | The initial viewport height for the headless layout pass.                                                    |
| `update_contract` | `"mobile-v1"`                 | The native compatibility contract named by code-pushed bundles in `runtime.json`.                            |

`native.package_id` defaults to `com.leptoskit.<name>`, with punctuation removed from the app name. Runtime plugin names belong in `native.plugins`. Each selected plugin must be a direct Cargo dependency of the selected platform's host package. See [automatic plugin registration](../platforms/host-projects.md#automatic-plugin-registration).

Each package id segment starts with an ASCII letter and contains only ASCII letters and digits. Change `native.update_contract` in a store release when plugin grants or native behavior change incompatibly. Shared app workspaces append the destination, such as `mobile-v1/ios`, so hosts reject another platform's update bundle.

## `[codepush]`

| Key           | Default        | Meaning                                                                                                                                                 |
| ------------- | -------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `enabled`     | `false`        | Whether the app checks for over-the-air updates.                                                                                                        |
| `server`      | `""`           | The release registry's base URL. Required when `enabled = true`.                                                                                        |
| `channel`     | `"production"` | The release channel this build tracks.                                                                                                                  |
| `mandatory`   | `false`        | Whether updates install without asking. The generated mobile hosts don't read it yet; they always wait for the user to activate.                        |
| `staging_dir` | `"codepush"`   | Where `CodePushClient::sync` stages updates. The generated mobile hosts keep releases in their own private, non-backed-up app directory instead.        |
| `public_keys` | `[]`           | The hex-encoded ed25519 publisher keys whose signed releases the app installs. At least one is required when `enabled = true`. List two to rotate keys. |
| `allow_http`  | `false`        | Lets debug native builds reach a plaintext `http://` registry, for local development. Release builds always require HTTPS.                              |

## `[router]`

The table's presence turns on [file routes](10-file-routes.md), even when it is empty. Without it, leptoskit leaves `src/routes/` alone.

| Key   | Default        | Meaning                                                                                                                                                                                                          |
| ----- | -------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `dir` | `"src/routes"` | The routes directory, relative to the project root. The generated module sits beside it with the same name, so `src/routes/` gets `src/routes.rs`. It must stay inside the project, written without `.` or `..`. |

## `[[plugins]]`

Each `[[plugins]]` table turns on one build plugin, which hooks into `leptoskit build` and `leptoskit dev`: it can generate code before cargo runs, adjust the build's cargo features, environment, and assets, run again before the dev and production servers start, answer its own `plugin:<name>` commands, and declare the native permissions and dependencies a generated host needs. These are not the runtime plugins that give native apps operating-system services; those are listed in `native.plugins` (see [runtime plugins](../platforms/plugins.md)).

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
