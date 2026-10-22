# Small bundles

Every byte of wasm is a byte the browser downloads, compiles, and runs before your page responds, and in a native app it is a byte that travels over the air with each update. leptos gives you two big levers for shipping less: render on the server so the client only hydrates, and hydrate only the islands that are interactive. leptoskit makes each one a line in `leptoskit.toml`, compiles the result as small as it goes, and tells you what you shipped.

## Pick a render mode

```toml
[app]
render = "islands"
```

| `render`    | What renders where                                              | What the wasm carries           |
| ----------- | --------------------------------------------------------------- | ------------------------------- |
| `"csr"`     | the browser renders everything                                  | the whole app                   |
| `"ssr"`     | the server renders every page, the browser hydrates all of it   | every component                 |
| `"islands"` | the server renders every page, the browser hydrates the islands | only the `#[island]` components |

`csr` is the default, so nothing changes until you opt in. The two server-rendering modes follow the convention every leptos app already uses: leptoskit builds the server package with its `ssr` feature and the wasm client with its `hydrate` feature, and in islands mode turns on `leptos/islands` for both. Since the mode is set in one place, the server and the client can't disagree about it.

## An islands app

The [`islands` example](https://github.com/ifiokjr/leptoskit/tree/main/examples/islands) is a complete app in one crate: the library is the wasm client and the binary is the server.

```toml
[lib]
crate-type = ["cdylib", "rlib"]

[[bin]]
name = "my_app"
path = "src/main.rs"
required-features = ["ssr"]

[dependencies]
leptos = { version = "0.8", features = ["islands"] }
leptos_router = "0.8"
serde = { version = "1", features = ["derive"] }
wasm-bindgen = "=0.2.129"
axum = { version = "0.8", optional = true }
leptos_axum = { version = "0.8", optional = true }
leptoskit_axum = { version = "0.1", optional = true }
tokio = { version = "1", features = ["rt-multi-thread", "macros"], optional = true }

[features]
ssr = [
	"dep:axum",
	"dep:leptos_axum",
	"dep:leptoskit_axum",
	"dep:tokio",
	"leptos/ssr",
	"leptos_router/ssr",
]
hydrate = ["leptos/hydrate"]
```

`render = "islands"` turns `leptos/islands` on for leptoskit's builds whatever the manifest says. Keep it in `Cargo.toml` too, because `cargo check` and rust-analyzer don't read `leptoskit.toml`, and `#[island]` doesn't compile without it. Island props cross from the server to the client as JSON, so the crate depends on `serde` directly.

The client entry point hydrates the islands, and nothing else:

```rust,ignore
#[cfg(feature = "hydrate")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn hydrate() {
	console_error_panic_hook::set_once();
	leptos::mount::hydrate_islands();
}
```

The shell asks `HydrationScripts` for the islands loader:

```rust,ignore
<HydrationScripts options islands=true />
```

The server renders the routes and serves the built site behind them with [`leptoskit_axum::static_files`](https://docs.rs/leptoskit_axum/latest/leptoskit_axum/fn.static_files.html):

```rust,ignore
let options = get_configuration(None)?.leptos_options;
let site_root = std::path::PathBuf::from(options.site_root.as_ref());
let app = Router::new()
	.leptos_routes(&options, generate_route_list(App), {
		let options = options.clone();
		move || shell(options.clone())
	})
	.fallback_service(leptoskit_axum::static_files(&site_root))
	.with_state(options);
```

`leptoskit dev` and `leptoskit serve` start that binary with `LEPTOS_OUTPUT_NAME`, `LEPTOS_SITE_ROOT`, `LEPTOS_SITE_PKG_DIR`, `LEPTOS_SITE_ADDR`, and `LEPTOS_ENV` set, so `get_configuration(None)` finds the built site with no `[package.metadata.leptos]` table. To run it yourself in production, set the same variables: `LEPTOS_SITE_ROOT` is the `--out` directory and `LEPTOS_OUTPUT_NAME` is the client's library name.

Mark a component `#[island]` only when it needs to run in the browser. Everything else, including the components that wrap islands, renders on the server and never reaches the wasm.

## Release builds

Everything about what a release build ships lives in one table:

```toml
[bundle]
optimize = "smallest" # or "fast", the default
precompress = true
budget_kib = 80
```

`leptoskit build --release` always does the cheap things that make the client smaller:

- **`wasm-opt -Oz`**, binaryen's most aggressive size optimization, when `bundle.wasm_opt` is on (the default).
- **No debugging sections.** The name and producers sections are dropped from release wasm.
- **Precompressed output.** Every compressible file (wasm, JavaScript, CSS, HTML, SVG, JSON) gets a brotli `.br` and a gzip `.gz` copy, compressed once at the highest settings instead of on every request. `leptoskit serve`, `leptoskit_axum::static_files`, and most CDNs send them to browsers that accept them. Turn it off with `bundle.precompress = false`. Dev builds remove the copies, so a server never sends a stale one.

`bundle.optimize` decides how much further to go. The trade-off is real, so it's your call:

| `optimize`         | The wasm client compiles with                                                      | Initial load (islands example) | Release rebuild (benchmark app) |
| ------------------ | ---------------------------------------------------------------------------------- | ------------------------------ | ------------------------------- |
| `"fast"` (default) | your `release` profile                                                             | 78.7 KiB                       | ~1.3 s                          |
| `"smallest"`       | `release` plus fat LTO, one codegen unit, `opt-level = "z"`, and `panic = "abort"` | 71.3 KiB (9% less)             | ~4 s                            |

Nearly all of `"smallest"`'s saving comes from fat LTO with a single codegen unit: the optimizer sees the whole program and drops what nothing reaches, but every release rebuild after a client change pays for it. Turn it on where the bytes matter, such as the deploy job, with `--optimize smallest`, or for every release build in `leptoskit.toml`. leptoskit defines the profile on cargo's command line, so your `Cargo.toml` doesn't change, and the server keeps your own `release` profile either way.

For full control, compile the client with a profile from your own `Cargo.toml` instead: `bundle.cargo_profile = "wasm-release"`.

## Native applications

A native app carries three things: the platform shell, the Rust runtime linked in as a static library — wasmi, the view layer, the layout engine, your plugins, and code push's TLS — and the wasm guest as an asset. Bytes cost more here than on the web: they ride the app-store download, the over-the-air update, and the device's storage.

`leptoskit native build --release` makes the cheap wins automatic:

- **`wasm-opt -Oz` over the guest**, exactly as the web client gets it, dropping the producers section nothing on a device reads (`bundle.wasm_opt`, on by default). Unlike the web client, a machine without binaryen still builds: the guest ships unoptimized with a warning instead of failing the build.
- **Stripped host code.** The generated Xcode project dead-strips and fully strips release apps; the macOS app is dead-stripped at link and stripped before signing; the Android app shrinks its Kotlin through R8 with keep rules for the classes the JNI library calls by name, which takes the dex of the scaffold from 3.1 MiB to under 150 KiB.
- **A size report.** Every platform build ends by walking the artifact it produced, so a size change surfaces in the build that caused it:

```text
application size (macos/App.app):
  Contents/MacOS/App                     3063.9 KiB
  Contents/Resources/app.wasm             116.1 KiB
  Contents/_CodeSignature/CodeResources     2.4 KiB
  Contents/Info.plist                       0.6 KiB
  total                                  3183.0 KiB
```

`bundle.optimize = "smallest"` reaches native builds too. The guest compiles with the same profile as the web client; the host library with a matching one that keeps unwinding, because the bridge turns plugin panics into native errors through `catch_unwind`, and `panic = "abort"` would take the whole app down instead. As on the web, the trade for the last kilobytes is runtime speed and rebuild time.

iOS and Android apps carry the guest once, gzipped inside the `app.lkb` code push baseline that updates are diffed against; macOS carries it as the raw `app.wasm`. [The size benchmark](../benchmarks.md#app-size) tracks the total for a minimal app on every platform.

## See what you ship

Every release build ends with the client bundle report. Here's the `islands` example, which builds with `optimize = "smallest"` and splits its lazy island off:

```text
client bundle (optimize = "smallest"):
                                                                 raw        gzip      brotli
  pkg/__wasm_split.______________________.js                 2.2 KiB     0.8 KiB     0.7 KiB
  pkg/islands_example.js                                    31.0 KiB     6.4 KiB     5.5 KiB
  pkg/islands_example_bg.wasm                              176.8 KiB    76.9 KiB    65.0 KiB
  pkg/split_mixer_loader_6820627641992162583.wasm (lazy)    44.2 KiB    27.5 KiB    24.8 KiB
  initial load                                             209.9 KiB    84.1 KiB    71.3 KiB  within the 80 KiB budget
```

The **initial load** is everything the page downloads before it can respond. Chunks marked `(lazy)` load later, only on pages that need them, so they don't count toward it. Set a budget for it and the build holds the line:

```toml
[bundle]
budget_kib = 80
```

A release build whose initial load is heavier than 80 KiB brotli-compressed fails, after writing its output, with the size it reached. Run `leptoskit build --release` in CI and a pull request that bloats the bundle can't merge unnoticed.

## Load code on demand

Islands keep static content out of the wasm. Code splitting goes further: an island, a route, or any function you mark lazy moves into a wasm chunk of its own, and the browser downloads it the first time a page needs it.

```toml
[bundle]
split = true
```

leptos has three ways to mark code lazy, and `split` handles all of them:

- **`#[island(lazy)]`**: the island's code loads when a page that renders it hydrates.
- **`#[lazy]`** on a function: its code loads the first time it's called.
- **Lazy routes**, `#[lazy_route]` on an `impl LazyRoute`: the route's data and view load when you navigate to it.

With `split` on, leptoskit links the client with `-Clink-args=--emit-relocs`, runs [wasm-split](https://github.com/WorldSEnder/wasm-split-prototype) over it in process, writes the chunks, their loader, and `__wasm_split_manifest.json` into `pkg/`, and only then runs wasm-bindgen. Release builds run `wasm-opt -Oz` over every chunk too. When the server calls a `#[lazy]` function while rendering, `HydrationScripts` reads the manifest and preloads that function's chunk. This needs `leptos_meta`'s `<MetaTags/>` in your shell. A lazy island's chunk loads while the island hydrates.

In the `islands` example, the colour mixer on `/lab` is `#[island(lazy)]`. With the default `optimize = "fast"`, building it as an ordinary island puts the initial load at 86.8 KiB brotli-compressed on every page. Split, the notes page loads 78.7 KiB and never fetches the mixer's 25.5 KiB chunk. That's 9% less on every page that doesn't use the mixer.

Lazy code needs splitting to run: without wasm-split, the client imports a chunk loader that doesn't exist, and the page fails to load. So a build with `#[lazy]` code and `split = false` stops and tells you to turn it on, rather than shipping a broken page.

> **Note:** `split` links the client with `-Clink-args=--emit-relocs` through cargo's configuration, which cargo ignores while the `RUSTFLAGS` environment variable is set. With `RUSTFLAGS` set, a split build fails unless the variable already includes `--emit-relocs`.
